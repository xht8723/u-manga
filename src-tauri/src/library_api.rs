use super::*;
use anyhow::Context;
use umanga_core::library::{self, Book, BookSummary, Chapter, ImportPreview, Metadata, Overrides};
fn authorize_assets(app: &tauri::AppHandle, book: &Path) -> Api<()> {
    let assets = store::assets(book);
    if assets.exists() {
        umanga_core::safety::no_link(&assets).map_err(error)?;
    }
    app.asset_protocol_scope()
        .allow_directory(assets, false)
        .map_err(error)
}
pub(super) fn root(state: &AppState) -> PathBuf {
    PathBuf::from(&state.settings.lock().library_directory)
}
#[tauri::command]
pub async fn library_list(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Api<Vec<BookSummary>> {
    let root = root(&state);
    if root.as_os_str().is_empty() {
        return Ok(vec![]);
    }
    let books = tauri::async_runtime::spawn_blocking(move || library::list(&root))
        .await
        .map_err(error)?
        .map_err(error)?;
    for book in &books {
        authorize_assets(&app, Path::new(&book.path))?;
    }
    Ok(books)
}
#[tauri::command]
pub async fn source_thumbnail(state: State<'_, AppState>, source: Source) -> Api<String> {
    let permit = state
        .thumbnails
        .clone()
        .acquire_owned()
        .await
        .map_err(error)?;
    let cache = state.thumbnail_cache.clone();
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<String> {
        let _permit = permit;
        cache.lock().get(&source)
    })
    .await
    .map_err(error)?
    .map_err(error)
}
#[tauri::command]
pub async fn book_open(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Api<Book> {
    require_managed(&state, &path)?;
    authorize_assets(&app, Path::new(&path))?;
    let root = root(&state);
    tauri::async_runtime::spawn_blocking(move || {
        library::open(Path::new(&path))
            .map(|book| library::refresh_committed(&root, Path::new(&path), book))
    })
    .await
    .map_err(error)?
    .map_err(error)
}
#[tauri::command]
pub async fn chapter_pages(
    state: State<'_, AppState>,
    path: String,
    chapter_id: String,
    app: tauri::AppHandle,
) -> Api<Project> {
    authorize_assets(&app, Path::new(&path))?;
    let _storage = state.storage_change.lock().await;
    require_managed(&state, &path)?;
    let defaults = state.settings.lock().clone();
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<Project> {
        let b = library::open(Path::new(&path))?;
        let settings = library::effective(&b, &defaults);
        store::settings(Path::new(&path), &settings)?;
        engine.recover(&path)?;
        Ok(Project {
            path: path.clone(),
            id: b.id,
            title: b
                .chapters
                .iter()
                .find(|c| c.id == chapter_id)
                .map(|c| c.title.clone())
                .unwrap_or(b.metadata.title),
            settings,
            omitted_page_ids: b.omitted_page_ids,
            pages: library::chapter_pages(Path::new(&path), &chapter_id)?,
        })
    })
    .await
    .map_err(error)?
    .map_err(error)
}
#[tauri::command]
pub async fn page_get(path: String, page_id: String) -> Api<Page> {
    tauri::async_runtime::spawn_blocking(move || store::page(Path::new(&path), &page_id))
        .await
        .map_err(error)?
        .map_err(error)
}
#[tauri::command]
pub async fn book_sources(
    state: State<'_, AppState>,
    path: String,
) -> Api<Vec<library::SourceIdentity>> {
    require_managed(&state, &path)?;
    tauri::async_runtime::spawn_blocking(move || library::source_identities(Path::new(&path)))
        .await
        .map_err(error)?
        .map_err(error)
}
#[tauri::command]
pub async fn import_scan(
    state: State<'_, AppState>,
    id: String,
    paths: Vec<String>,
    automatic: bool,
) -> Api<ImportPreview> {
    let storage = state.storage_change.lock().await;
    let token = CancellationToken::new();
    if state.downloads.lock().contains_key(&format!("scan:{id}")) {
        return Err("This scan is already running".into());
    }
    state
        .downloads
        .lock()
        .insert(format!("scan:{id}"), token.clone());
    drop(storage);
    let result =
        tauri::async_runtime::spawn_blocking(move || library::scan(&paths, automatic, &token))
            .await
            .map_err(error);
    state.downloads.lock().remove(&format!("scan:{id}"));
    result?.map_err(error)
}
#[tauri::command]
pub fn import_cancel(state: State<AppState>, id: String) {
    if let Some(t) = state.downloads.lock().get(&format!("scan:{id}")) {
        t.cancel()
    }
}
#[tauri::command]
pub async fn book_create(
    state: State<'_, AppState>,
    metadata: Metadata,
    preview: ImportPreview,
    cover: Option<String>,
) -> Api<Book> {
    let _storage = state.storage_change.lock().await;
    let root = root(&state);
    let result = tauri::async_runtime::spawn_blocking(move || {
        library::create(&root, metadata, preview, cover)
    })
    .await
    .map_err(error)?
    .map_err(error)?;
    hosting::notify_clients(&state.app, &result.path, None);
    Ok(result)
}
#[tauri::command]
pub async fn book_import(state: State<'_, AppState>, path: String) -> Api<Book> {
    let _storage = state.storage_change.lock().await;
    let root = root(&state);
    let engine = state.engine.clone();
    let hub = state.jobs_hub.clone();
    let result = tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<Book> {
        let book = library::import_book(&root, Path::new(&path))?;
        if let Err(error) = engine.recover(&book.path) {
            hub.record_error(format!("{}: {error}", book.metadata.title));
        }
        hub.invalidate(&book.path, &engine);
        Ok(book)
    })
    .await
    .map_err(error)?
    .map_err(error)?;
    hosting::notify_clients(&state.app, &result.path, None);
    Ok(result)
}
#[tauri::command]
pub async fn book_update(
    state: State<'_, AppState>,
    path: String,
    metadata: Metadata,
    overrides: Overrides,
    expected: u64,
    cover_changed: Option<bool>,
    cover: Option<String>,
) -> Api<Book> {
    let _storage = state.storage_change.lock().await;
    require_managed(&state, &path)?;
    let root = root(&state);
    let hub = state.jobs_hub.clone();
    let engine = state.engine.clone();
    let result = tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<Book> {
        let book = library::update_with_cover(
            &root,
            Path::new(&path),
            metadata,
            overrides,
            expected,
            cover_changed.unwrap_or(false).then_some(cover.as_deref()),
        )?;
        hub.invalidate(&path, &engine);
        Ok(book)
    })
    .await
    .map_err(error)?
    .map_err(error)?;
    hosting::notify_clients(&state.app, &result.path, None);
    Ok(result)
}
#[tauri::command]
pub async fn chapter_complete(
    state: State<'_, AppState>,
    path: String,
    chapter_id: String,
    read: bool,
) -> Api<Book> {
    let _storage = state.storage_change.lock().await;
    require_managed(&state, &path)?;
    let root = root(&state);
    let result = tauri::async_runtime::spawn_blocking(move || {
        library::complete(&root, Path::new(&path), &chapter_id, read)
    })
    .await
    .map_err(error)?
    .map_err(error)?;
    hosting::notify_clients(&state.app, &result.path, None);
    Ok(result)
}
#[tauri::command]
pub async fn book_organize(
    state: State<'_, AppState>,
    path: String,
    expected: u64,
    chapters: Vec<Chapter>,
    added: Vec<Page>,
    omitted_page_ids: Vec<String>,
) -> Api<Book> {
    let _storage = state.storage_change.lock().await;
    require_managed(&state, &path)?;
    let _reservation = state.engine.reserve_book(&path).await.map_err(error)?;
    let root = root(&state);
    let p = path.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        library::organize(
            &root,
            Path::new(&p),
            expected,
            chapters,
            added,
            omitted_page_ids,
        )
    })
    .await
    .map_err(error)
    .and_then(|r| r.map_err(error));
    let engine = state.engine.clone();
    let hub = state.jobs_hub.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let keep = library::open(Path::new(&path)).ok().map(|b| {
            b.chapters
                .into_iter()
                .flat_map(|c| c.page_ids)
                .collect::<Vec<_>>()
        });
        engine.release_book(&path, keep.as_deref());
        hub.invalidate(&path, &engine);
    })
    .await
    .map_err(error)?;
    if let Ok(book) = &result {
        hosting::notify_clients(&state.app, &book.path, None);
    }
    result
}
#[tauri::command]
pub async fn book_delete(state: State<'_, AppState>, path: String) -> Api<()> {
    let _storage = state.storage_change.lock().await;
    require_managed(&state, &path)?;
    let _reservation = state.engine.reserve_book(&path).await.map_err(error)?;
    let root = root(&state);
    let engine = state.engine.clone();
    let hub = state.jobs_hub.clone();
    let notify_path = path.clone();
    let result = tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<()> {
        let result = library::delete(&root, Path::new(&path));
        let keep = if result.is_err() {
            library::open(Path::new(&path)).ok().map(|b| {
                b.chapters
                    .into_iter()
                    .flat_map(|c| c.page_ids)
                    .collect::<Vec<_>>()
            })
        } else {
            None
        };
        engine.release_book(&path, keep.as_deref());
        hub.invalidate(&path, &engine);
        result
    })
    .await
    .map_err(error)?
    .map_err(error);
    if result.is_ok() {
        hosting::notify_clients(&state.app, &notify_path, None);
    }
    result
}
#[tauri::command]
pub async fn book_refresh(path: String) -> Api<BookSummary> {
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<_> {
        let path = Path::new(&path);
        library::refresh(path.parent().context("Book has no library folder")?, path)
    })
    .await
    .map_err(error)?
    .map_err(error)
}
#[tauri::command]
pub fn show_source(path: String) -> Api<()> {
    let path = Path::new(&path);
    let folder = if path.is_dir() {
        path
    } else {
        path.parent().ok_or("Source has no folder")?
    };
    #[cfg(windows)]
    {
        std::process::Command::new("explorer.exe")
            .arg(folder)
            .spawn()
            .map_err(error)?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(folder)
            .spawn()
            .map_err(error)?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(folder)
            .spawn()
            .map_err(error)?;
    }
    Ok(())
}
#[tauri::command]
pub async fn book_export(
    state: State<'_, AppState>,
    path: String,
    chapter_id: Option<String>,
    destination: String,
    format: String,
) -> Api<usize> {
    let _storage = state.storage_change.lock().await;
    require_managed(&state, &path)?;
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<usize> {
        let ((b, mut p), _assets) = umanga_core::assets::protect_snapshot(|| {
            let b = library::open(Path::new(&path))?;
            let p = store::open(Path::new(&path))?;
            let paths = p
                .pages
                .iter()
                .flat_map(umanga_core::assets::page_references)
                .collect();
            Ok(((b, p), paths))
        })?;
        let mut names = HashMap::new();
        for (i, c) in b
            .chapters
            .iter()
            .enumerate()
            .filter(|(_, c)| chapter_id.as_ref().is_none_or(|id| id == &c.id))
        {
            let title: String = c
                .title
                .chars()
                .map(|ch| {
                    if ch.is_control() || "<>:\"/\\|?*".contains(ch) {
                        '_'
                    } else {
                        ch
                    }
                })
                .collect();
            let title = title.trim_end_matches([' ', '.']);
            for (j, id) in c.page_ids.iter().enumerate() {
                names.insert(
                    id.clone(),
                    if chapter_id.is_some() {
                        format!("{:04}.png", j + 1)
                    } else {
                        format!("{:03} {}/{:04}.png", i + 1, title, j + 1)
                    },
                );
            }
        }
        p.pages.retain(|p| names.contains_key(&p.id));
        documents::export_named(&p, Path::new(&destination), &format, &names)
    })
    .await
    .map_err(error)?
    .map_err(error)
}
#[tauri::command]
pub async fn library_clear_thumbnails(state: State<'_, AppState>) -> Api<usize> {
    let cache = state.thumbnail_cache.clone();
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<usize> { cache.lock().clear() })
        .await
        .map_err(error)?
        .map_err(error)
}
#[tauri::command]
pub fn show_notices(app: tauri::AppHandle) -> Api<()> {
    let folder = app.path().app_cache_dir().map_err(error)?;
    let native = runtime::extract(&folder).map_err(error)?;
    show_source(native.join("licenses").to_string_lossy().into())
}
