#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod hosting;
use hosting::{host_qr, host_start, host_status, host_stop};
mod batch_api;
mod glossary_api;
use batch_api::{translation_batch_preview, translation_batch_submit};
use glossary_api::{book_glossary_save, glossary_export, glossary_parse};
mod jobs_api;
mod jobs_smoke;
use jobs_smoke::jobs_smoke_ui_report;
mod library_api;
mod relocation;
use jobs_api::*;
use relocation::library_relocate;
mod locale;
mod region_api;
mod requirements_api;
use region_api::{region_cancel, region_read, region_translate};
mod runtime;
use locale::{onboarding_language, system_language};
mod setup_api;
mod window_state;
use library_api::*;
use parking_lot::Mutex;
use requirements_api::{action_readiness, jobs_readiness, prepare_enqueue, restart_enqueue};
use setup_api::*;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use tauri::{Emitter, Manager, State};
use tokio_util::sync::CancellationToken;
use umanga_core::{
    documents, inference, models, pipeline::Engine, providers, store, thumbnails, types::*,
};
type Api<T> = Result<T, umanga_core::ui_message::UiMessage>;
fn error(e: impl std::fmt::Display) -> umanga_core::ui_message::UiMessage {
    umanga_core::ui_message::UiMessage::error_text(e.to_string())
}
struct AppState {
    app: tauri::AppHandle,
    host: Arc<hosting::HostManager>,
    settings: Mutex<AppSettings>,
    folder: PathBuf,
    engine: Arc<Engine>,
    jobs_hub: Arc<JobsHub>,
    downloads: Mutex<HashMap<String, CancellationToken>>,
    storage_change: tokio::sync::Mutex<()>,
    thumbnails: Arc<tokio::sync::Semaphore>,
    thumbnail_cache: Arc<Mutex<thumbnails::Cache>>,
    bundled_models: PathBuf,
    verification: Arc<Mutex<models::VerificationCache>>,
}
fn require_managed(state: &AppState, path: &str) -> Api<()> {
    let root = PathBuf::from(&state.settings.lock().library_directory);
    if root.as_os_str().is_empty() {
        return Err("Choose a library first".into());
    }
    let target = umanga_core::safety::resolved(Path::new(path)).map_err(error)?;
    let root = root.canonicalize().map_err(error)?;
    if target.parent() != Some(root.as_path())
        || target.extension().and_then(|v| v.to_str()) != Some("umanga")
    {
        return Err("Book does not belong to the active library; reopen it from Library".into());
    }
    Ok(())
}
async fn persist_preferences(state: &AppState, settings: &AppSettings) -> Api<()> {
    for profile in &settings.providers {
        profile.instructions.validate().map_err(error)?;
    }
    let bytes = serde_json::to_vec_pretty(settings).map_err(error)?;
    let path = state.folder.join("preferences.json");
    tauri::async_runtime::spawn_blocking(move || store::atomic_write(&path, &bytes))
        .await
        .map_err(error)?
        .map_err(error)?;
    *state.settings.lock() = settings.clone();
    Ok(())
}
fn credential(p: &ProviderProfile) -> anyhow::Result<keyring::Entry> {
    Ok(keyring::Entry::new(
        "U-Manga",
        &providers::credential_scope(p)?,
    )?)
}
#[tauri::command]
fn bootstrap(state: State<AppState>) -> serde_json::Value {
    jobs_smoke::trace("bootstrap start");
    let result = serde_json::json!({"systemLocale":locale::system_locale(),"jobsSmoke":jobs_smoke::requested(),"settings":*state.settings.lock(),"models":models::catalog().unwrap_or_default(),"catalog":providers::cached_catalog(&state.folder),"fonts":state.engine.renderer.font_names()});
    jobs_smoke::trace("bootstrap complete");
    result
}
#[tauri::command]
async fn preferences(
    state: State<'_, AppState>,
    settings: AppSettings,
    base: Option<AppSettings>,
) -> Api<AppSettings> {
    let _storage = state.storage_change.lock().await;
    let current = state.settings.lock().clone();
    let mut settings =
        umanga_core::preferences::merge(base.as_ref().unwrap_or(&current), &settings, &current)
            .map_err(error)?;
    settings.setup = current.setup;
    apply_preferences_locked(&state, settings).await
}
async fn apply_preferences_locked(state: &AppState, settings: AppSettings) -> Api<AppSettings> {
    if settings.format_version != 15 {
        return Err("Unsupported preferences format; use a fresh installation profile.".into());
    }
    if !(1..=4).contains(&settings.concurrent_books) {
        return Err("Use 1–4 concurrent books.".into());
    }
    if !["day", "night"].contains(&settings.appearance.active.as_str()) {
        return Err("Invalid appearance profile".into());
    }
    for profile in &settings.providers {
        profile.instructions.validate().map_err(error)?;
    }
    if settings.setup.completed {
        for p in &settings.providers {
            providers::credential_scope(p).map_err(error)?;
        }
    }
    let changing_library = state.settings.lock().library_directory != settings.library_directory;
    if changing_library && state.folder.join("relocation.json").exists() {
        return Err(
            "Finish the interrupted library move using Move library before switching libraries."
                .into(),
        );
    }
    if changing_library && !state.downloads.lock().is_empty() {
        return Err("Finish or cancel model downloads before changing the library.".into());
    }
    if changing_library && !settings.library_directory.is_empty() {
        let root = PathBuf::from(&settings.library_directory);
        tauri::async_runtime::spawn_blocking(move || umanga_core::library::list(&root))
            .await
            .map_err(error)?
            .map_err(error)?;
    }
    let mut reservations = Vec::new();
    let mut old_paths = Vec::new();
    if changing_library {
        state.jobs_hub.wait_recovery().await;
        let paths: std::collections::HashSet<_> = state.engine.operation_books();
        for path in paths {
            reservations.push(state.engine.reserve_book(&path).await.map_err(error)?);
            old_paths.push(path);
        }
    }
    let previous = state.settings.lock().clone();
    let root = PathBuf::from(&settings.library_directory);
    let models = settings.model_directory();
    persist_preferences(state, &settings).await?;
    state.engine.configure(&state.settings.lock());
    if previous.model_directory() != models {
        state.engine.inference.lock().set_root(models.clone());
        *state.engine.model_directory.lock() = models;
    }
    if changing_library {
        state.host.stop();
        for path in old_paths {
            state.engine.release_book(&path, None);
        }
        drop(reservations);
        state.jobs_hub.activate(root, state.engine.clone());
    }
    state.host.publish_requirements();
    Ok(settings)
}
#[tauri::command]
async fn secret_set(
    state: State<'_, AppState>,
    profile: ProviderProfile,
    value: String,
) -> Api<()> {
    if !providers::credential_required(&profile) {
        return Err("Ollama does not require a saved credential.".into());
    }
    if value.trim().is_empty() {
        return Err("Enter a credential before saving.".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        credential(&profile)
            .map_err(error)?
            .set_password(&value)
            .map_err(error)
    })
    .await
    .map_err(error)??;
    state.host.publish_requirements();
    Ok(())
}
#[tauri::command]
async fn secret_status(profile: ProviderProfile) -> Api<bool> {
    if !providers::credential_required(&profile) {
        return Ok(false);
    }
    tauri::async_runtime::spawn_blocking(move || {
        Ok(credential(&profile).map_err(error)?.get_password().is_ok())
    })
    .await
    .map_err(error)?
}
#[tauri::command]
async fn page_image(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
    page_id: String,
    translated: bool,
) -> Api<String> {
    require_managed(&state, &path)?;
    let file = tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<String> {
        let page = store::page(Path::new(&path), &page_id)?;
        documents::source_health(&page)?;
        if translated
            && let Some(p) = &page.rendered
            && Path::new(p).exists()
        {
            return Ok(p.clone());
        }
        Ok(documents::materialize(Path::new(&path), &page)?
            .to_string_lossy()
            .into())
    })
    .await
    .map_err(error)?
    .map_err(error)?;
    app.asset_protocol_scope()
        .allow_file(&file)
        .map_err(error)?;
    Ok(file)
}
#[tauri::command]
async fn thumbnail(state: State<'_, AppState>, path: String, page_id: String) -> Api<String> {
    let permit = state
        .thumbnails
        .clone()
        .acquire_owned()
        .await
        .map_err(error)?;
    let cache = state.thumbnail_cache.clone();
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<String> {
        let _permit = permit;
        let page = store::page(Path::new(&path), &page_id)?;
        cache.lock().get(&page.source)
    })
    .await
    .map_err(error)?
    .map_err(error)
}
#[tauri::command]
async fn edit_page(
    state: State<'_, AppState>,
    path: String,
    page: Page,
    base: Page,
    expected: u64,
) -> Api<Page> {
    commit_edit(&state, path, page, base, expected, false).await
}
async fn commit_edit(
    state: &AppState,
    path: String,
    page: Page,
    base: Page,
    expected: u64,
    strict: bool,
) -> Api<Page> {
    let _storage = state.storage_change.lock().await;
    require_managed(state, &path)?;
    let engine = state.engine.clone();
    let defaults = state.settings.lock().clone();
    let guard = engine
        .pause_for_edit(&path, &page.id)
        .await
        .map_err(error)?;
    let verification = state.verification.clone();
    let bundled = state.bundled_models.clone();
    let notify_path = path.clone();
    let result = tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<Page> {
        let _guard = guard;
        let _outputs = umanga_core::assets::PageOutputs::new(Path::new(&path), &page.id)?;
        anyhow::ensure!(base.revision == expected, "Editor baseline changed");
        let latest = store::page(Path::new(&path), &page.id)?;
        if strict && latest.revision != expected {
            anyhow::bail!("Page changed; refresh and review your draft before saving.");
        }
        let expected = latest.revision;
        let mut page = umanga_core::editing::merge(&base, &page, &latest)?;
        let im = documents::load(&page.source)?;
        documents::source_health(&page)?;
        for r in &mut page.regions {
            if r.bbox.iter().any(|x| !x.is_finite())
                || r.bbox[2] <= r.bbox[0]
                || r.bbox[3] <= r.bbox[1]
            {
                anyhow::bail!("Regions need a positive width and height");
            }
            r.bbox = inference::clamp(r.bbox, &im);
            if latest
                .regions
                .iter()
                .find(|old| old.id == r.id)
                .is_none_or(|old| old.bbox != r.bbox)
            {
                umanga_core::editing::validate_balloon(r);
            }
        }
        let book = umanga_core::library::open(Path::new(&path))?;
        let settings = umanga_core::library::effective(&book, &defaults);
        let mut rendering_settings = settings.clone();
        if let Some(saved) = &page.cleanup {
            rendering_settings.cleanup = saved.settings.clone();
        }
        let cached = umanga_core::requirements::cached_edit(
            &page,
            &latest,
            &engine.renderer,
            &settings.target_language,
        );
        let mut cache = verification.lock().clone();
        let availability = umanga_core::requirements::check(
            umanga_core::requirements::Action::Edit,
            &rendering_settings,
            None,
            false,
            &models::ModelLocations {
                bundled,
                downloaded: defaults.model_directory(),
            },
            &mut cache,
            cached,
        )?;
        anyhow::ensure!(availability.ready, "{}", availability.reason());
        let clean = page
            .background
            .as_ref()
            .map(|path| umanga_core::image_input::open(Path::new(path)))
            .transpose()?;
        let mut background_output = None;
        if let Some(c) = &clean {
            if c.width() != im.width() || c.height() != im.height() {
                anyhow::bail!("Cleaned background dimensions must match the original")
            }
            let background = store::assets(Path::new(&path))
                .join(format!("clean-{}.png", store::digest(c.to_rgb8().as_raw())));
            background_output = Some(umanga_core::assets::ProvisionalOutput::image(
                Path::new(&path),
                &background,
                c,
            )?);
            page.background = Some(background.to_string_lossy().into());
        }
        let rendered = engine.render_page(
            &im,
            &mut page,
            &settings,
            Path::new(&path),
            &defaults.model_directory(),
            false,
            &CancellationToken::new(),
            Arc::new(|_, _, _| {}),
        )?;
        let output =
            umanga_core::safety::page_asset(Path::new(&path), &page.id, &format!("{}.png", uid()))?;
        umanga_core::assets::save_image(&rendered, &output)?;
        page.rendered = Some(output.to_string_lossy().into());
        page.status = if page.regions.iter().any(|r| r.review.is_some()) {
            "review"
        } else {
            "edited"
        }
        .into();
        if !store::save_page(Path::new(&path), &mut page, expected)? {
            anyhow::bail!("Page changed while editing; reload before applying")
        };
        if let Some(output) = &mut background_output {
            output.commit();
        }
        Ok(page)
    })
    .await
    .map_err(error)?
    .map_err(error)?;
    hosting::notify_clients(&state.app, &notify_path, Some(&result.id));
    Ok(result)
}
#[tauri::command]
async fn enqueue(
    state: State<'_, AppState>,
    path: String,
    page_ids: Vec<String>,
    priority: Option<bool>,
) -> Api<umanga_core::queue::Submission> {
    let _storage = state.storage_change.lock().await;
    require_managed(&state, &path)?;
    let mut settings = state.settings.lock().clone();
    let lookup = path.clone();
    let defaults = settings.clone();
    settings.translation = tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<_> {
        let book = umanga_core::library::open(Path::new(&lookup))?;
        Ok(umanga_core::library::effective(&book, &defaults))
    })
    .await
    .map_err(error)?
    .map_err(error)?;
    let readiness = requirements_api::applied_action(
        &state,
        &settings,
        umanga_core::requirements::Action::Translate,
        false,
    )
    .await?;
    if !readiness.ready {
        return Err(readiness
            .issues
            .iter()
            .map(|i| i.message.as_str())
            .collect::<Vec<_>>()
            .join("\n")
            .into());
    }
    let mut profile = settings
        .providers
        .iter()
        .find(|p| p.id == settings.translation.provider_id)
        .cloned()
        .ok_or("Select a translation provider in Settings")?;
    profile.thinking_policy = Some(umanga_core::thinking::resolve(&profile).await);
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || -> Api<_> {
        let jobs = engine
            .enqueue_captured(
                &path,
                &page_ids,
                &profile,
                &settings.translation,
                JobKind::Translation,
                &settings.model_directory(),
            )
            .map_err(error)?;
        if priority == Some(true) {
            engine.prioritize(&path, &page_ids);
        }
        Ok(umanga_core::queue::submission(jobs, engine.is_held()))
    })
    .await
    .map_err(error)?
}
#[tauri::command]
async fn model_verify(state: State<'_, AppState>, id: String, library: String) -> Api<bool> {
    let settings = state.settings.lock().clone();
    require_applied_library(&settings, &library)?;
    let locations = model_locations(&state, &settings);
    let cache = state.verification.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let pack = models::catalog()?
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| anyhow::anyhow!("Unknown model"))?;
        let mut verification = cache.lock().clone();
        verification.check(locations.root(&pack), &pack, true)
    })
    .await
    .map_err(error)?
    .map_err(error)
}
#[tauri::command]
async fn model_download(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
    library: String,
) -> Api<()> {
    let storage = state.storage_change.lock().await;
    let pack = models::catalog()
        .map_err(error)?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or("Unknown model")?;
    if pack.distribution == "bundled" {
        return Err(
            "Dialogue detection is included. Restart U-Manga to repair its cached copy.".into(),
        );
    }
    let settings = state.settings.lock().clone();
    require_applied_library(&settings, &library)?;
    let root = settings.model_directory();
    let token = CancellationToken::new();
    if state.downloads.lock().contains_key(&id) {
        return Err("Download already running".into());
    }
    state.downloads.lock().insert(id.clone(), token.clone());
    drop(storage);
    let result = models::download(&root, &pack, &token, |done, total| {
        let _ = app.emit(
            "download",
            serde_json::json!({"id":id,"done":done,"total":total}),
        );
    })
    .await
    .map_err(error);
    state.downloads.lock().remove(&id);
    let _ = app.emit("download-ended", serde_json::json!({"id":id}));
    state.host.publish_requirements();
    result
}
#[tauri::command]
fn model_cancel(state: State<AppState>, id: String) {
    if let Some(t) = state.downloads.lock().get(&id) {
        t.cancel()
    }
}
#[tauri::command]
async fn catalog_refresh(state: State<'_, AppState>) -> Api<serde_json::Value> {
    providers::refresh_catalog(&state.folder)
        .await
        .map_err(error)
}

fn main() {
    if !runtime::check_webview() {
        return;
    }
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            #[cfg(windows)]
            if let Some(window) = app.get_webview_window("main") {
                window.with_webview(|webview| unsafe {
                    let result = webview
                        .controller()
                        .CoreWebView2()
                        .and_then(|view| view.Settings())
                        .and_then(|settings| settings.SetAreDefaultContextMenusEnabled(false));
                    if let Err(error) = result {
                        eprintln!("Could not disable default context menus: {error}");
                    }
                })?;
            }
            let test_root = std::env::var_os("U_MANGA_DATA_DIR").map(PathBuf::from);
            let folder = test_root.clone().unwrap_or(app.path().app_config_dir()?);
            std::fs::create_dir_all(&folder)?;
            if let Err(error) = window_state::install(app, folder.clone()) {
                eprintln!("Could not restore window bounds: {error}");
            }
            let mut settings: AppSettings = match std::fs::read(folder.join("preferences.json")) {
                Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("Unsupported preferences format. Existing data was preserved; use a fresh installation profile."))?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => AppSettings::default(),
                Err(e) => return Err(e.into()),
            };
            if settings.format_version != 15 { return Err("Unsupported preferences format; existing data was preserved.".into()); }
            for profile in &settings.providers { profile.instructions.validate()?; }
            if jobs_smoke::requested() {
                settings = jobs_smoke::prepare()?;
                store::atomic_write(
                    &folder.join("preferences.json"),
                    &serde_json::to_vec_pretty(&settings)?,
                )?;
            }
            let cache = test_root
                .map(|p| p.join("cache"))
                .unwrap_or(app.path().app_cache_dir()?);
            let thumbnail_cache = thumbnails::Cache::new(cache.join("thumbnails-v1"))?;
            let native = runtime::extract(&cache)?;
            runtime::prepare_loader(&native.join("runtime"))?;
            inference::init(&native.join("runtime"))?;
            documents::init_pdfium(&native.join("runtime"))?;
            let handle = app.handle().clone();
            let jobs_hub = Arc::new(JobsHub::default());
            let notifications = jobs_hub.clone();
            let bundled_models = native.join("models");
            let verification = Arc::new(Mutex::new(models::VerificationCache::default()));
            let engine = Engine::with_locations(
                models::ModelLocations {
                    bundled: bundled_models.clone(),
                    downloaded: settings.model_directory(),
                },
                &native.join("fonts"),
                Arc::new(move |j| {
                    notifications.dirty(j.id);
                }),
                Arc::new(|profile| {
                    credential(profile)?.get_password().map_err(|_| {
                        anyhow::anyhow!("Enter an API key for this provider and endpoint")
                    })
                }),
            )?;
            let guard_models = bundled_models.clone();
            let guard_cache = verification.clone();
            engine.set_requirements(Arc::new(move |job| {
                let models = guard_models.clone();
                let cache = guard_cache.clone();
                Box::pin(async move {
                    let status = requirements_api::job_availability(models, cache, job)
                        .await
                        .map_err(anyhow::Error::msg)?;
                    anyhow::ensure!(status.ready, "{}", status.reason());
                    Ok(())
                })
            }));
            engine.configure(&settings);
            let worker = engine.clone();
            tauri::async_runtime::spawn(async move { worker.start() });
            jobs_hub.start(handle, engine.clone());
            jobs_hub.activate(PathBuf::from(&settings.library_directory), engine.clone());
            let host = Arc::new(hosting::HostManager::new(&folder));
            app.manage(AppState {
                app: app.handle().clone(),
                host,
                settings: Mutex::new(settings),
                folder,
                engine,
                jobs_hub,
                downloads: Mutex::new(HashMap::new()),
                storage_change: tokio::sync::Mutex::new(()),
                thumbnails: Arc::new(tokio::sync::Semaphore::new(2)),
                thumbnail_cache: Arc::new(Mutex::new(thumbnail_cache)),
                bundled_models,
                verification,
            });
            jobs_smoke::launch(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            host_start,
            host_stop,
            host_status,
            host_qr,
            system_language,
            onboarding_language,
            library_list,
            book_open,
            chapter_pages,
            book_sources,
            import_scan,
            import_cancel,
            book_create,
            book_import,
            book_update,
            book_glossary_save,
            glossary_parse,
            glossary_export,
            chapter_complete,
            book_organize,
            book_omissions_update,
            book_delete,
            book_refresh,
            show_source,
            book_export,
            library_relocate,
            library_clear_thumbnails,
            show_notices,
            source_thumbnail,
            preferences,
            translation_readiness,
            action_readiness,
            jobs_readiness,
            prepare_enqueue,
            restart_enqueue,
            translation_batch_preview,
            translation_batch_submit,
            region_read,
            region_translate,
            region_cancel,
            ollama_models,
            ollama_model_details,
            thinking_capability,
            onboarding_save,
            model_storage,
            service_test,
            prompt_defaults,
            prompt_preview,
            onboarding_appearance,
            model_downloads,
            secret_set,
            secret_status,
            page_image,
            page_get,
            thumbnail,
            edit_page,
            enqueue,
            cleanup_enqueue,
            job_control,
            jobs_control_all,
            jobs_list,
            jobs_smoke_ui_report,
            model_verify,
            model_download,
            model_cancel,
            catalog_refresh,
        ])
        .build(tauri::generate_context!());
    match result {
        Ok(app) => app.run(|app, event| {
            if matches!(
                event,
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
            ) && let Some(state) = app.try_state::<AppState>()
            {
                state.host.stop();
            }
        }),
        Err(e) => runtime::startup_error(&e.to_string()),
    }
}
