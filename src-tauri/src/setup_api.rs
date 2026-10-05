use super::*;
use umanga_core::setup;

#[tauri::command]
pub fn prompt_defaults(source_language: String) -> Api<umanga_core::instructions::Overrides> {
    if !setup::SOURCES.contains(&source_language.as_str()) {
        return Err("Select supported source and target languages.".into());
    }
    Ok(umanga_core::instructions::defaults(&source_language))
}
#[tauri::command]
pub fn prompt_preview(
    profile: ProviderProfile,
    settings: TranslationSettings,
    task: umanga_core::instructions::Task,
) -> Api<umanga_core::instructions::Preview> {
    if !setup::SOURCES.contains(&settings.source_language.as_str())
        || !setup::TARGETS.contains(&settings.target_language.as_str())
    {
        return Err("Select supported source and target languages.".into());
    }
    umanga_core::instructions::preview(&profile, &settings, task).map_err(error)
}

pub(super) fn model_locations(state: &AppState, settings: &AppSettings) -> models::ModelLocations {
    models::ModelLocations {
        bundled: state.bundled_models.clone(),
        downloaded: settings.model_directory(),
    }
}
pub(super) fn has_credential(settings: &AppSettings) -> bool {
    settings
        .providers
        .iter()
        .find(|p| p.id == settings.translation.provider_id)
        .filter(|p| providers::credential_required(p))
        .and_then(|p| credential(p).ok())
        .and_then(|e| e.get_password().ok())
        .is_some_and(|k| !k.trim().is_empty())
}
pub(super) async fn report(
    state: &AppState,
    settings: AppSettings,
    force: bool,
) -> Api<setup::Readiness> {
    let locations = model_locations(state, &settings);
    let cache = state.verification.clone();
    let local = settings.clone();
    let mut readiness = tauri::async_runtime::spawn_blocking(move || {
        let mut verification = cache.lock().clone();
        setup::readiness(
            &local,
            &locations,
            &mut verification,
            has_credential(&local),
            force,
        )
    })
    .await
    .map_err(error)?
    .map_err(error)?;
    setup::check_service(&mut readiness, &settings, force).await;
    Ok(readiness)
}
#[tauri::command]
pub async fn translation_readiness(
    state: State<'_, AppState>,
    settings: AppSettings,
    force: Option<bool>,
) -> Api<setup::Readiness> {
    report(&state, settings, force.unwrap_or(false)).await
}
#[tauri::command]
pub async fn onboarding_save(
    state: State<'_, AppState>,
    mut settings: AppSettings,
    base: Option<AppSettings>,
    step: String,
    complete: bool,
) -> Api<AppSettings> {
    let _storage = state.storage_change.lock().await;
    let current_settings = state.settings.lock().clone();
    settings = umanga_core::preferences::merge(
        base.as_ref().unwrap_or(&current_settings),
        &settings,
        &current_settings,
    )
    .map_err(error)?;
    if state.settings.lock().setup.completed {
        return Err("Setup is already complete. Use Settings to make changes.".into());
    }
    let order = ["library", "pipeline", "services", "models", "review"];
    let current = state.settings.lock().setup.step.clone();
    let next_index = order
        .iter()
        .position(|s| *s == step)
        .ok_or("Unknown setup step")?;
    let going_back =
        !complete && next_index < order.iter().position(|s| *s == current).unwrap_or(0);
    if !going_back {
        let readiness = report(&state, settings.clone(), false).await?;
        setup::validate_step(&readiness, &step, complete, &settings.translation.mode)
            .map_err(error)?;
    }
    if !going_back && (step != "library" || complete) {
        let root = PathBuf::from(&settings.library_directory);
        tauri::async_runtime::spawn_blocking(move || {
            umanga_core::library::validate_location(&root)
        })
        .await
        .map_err(error)?
        .map_err(error)?;
    }
    settings.setup = SetupState {
        completed: complete,
        step,
    };
    apply_preferences_locked(&state, settings).await
}
pub(super) fn require_applied_library(settings: &AppSettings, library: &str) -> Api<()> {
    if settings.library_directory.trim().is_empty() {
        return Err("Choose and apply a library folder before downloading models.".into());
    }
    if settings.library_directory != library {
        return Err("Apply the library change before downloading or verifying models.".into());
    }
    if !Path::new(library).is_dir() {
        return Err(
            "The library folder is unavailable. Reconnect it or choose another library.".into(),
        );
    }
    Ok(())
}
#[tauri::command]
pub fn model_storage(state: State<AppState>) -> serde_json::Value {
    let settings = state.settings.lock();
    serde_json::json!({ "library": settings.library_directory, "directory": settings.model_directory() })
}
#[tauri::command]
pub async fn service_test(settings: AppSettings) -> Api<setup::ServiceTest> {
    let p = settings
        .providers
        .iter()
        .find(|p| p.id == settings.translation.provider_id)
        .ok_or("Select a translation service.")?;
    let key = if providers::credential_required(p) {
        let profile = p.clone();
        tauri::async_runtime::spawn_blocking(move || -> Api<String> {
            credential(&profile)
                .map_err(error)?
                .get_password()
                .map_err(|_| "Enter an API key for this service and endpoint.".into())
        })
        .await
        .map_err(error)??
    } else {
        String::new()
    };
    setup::test_service(p, &key, &settings.translation)
        .await
        .map_err(error)
}
#[tauri::command]
pub async fn thinking_capability(profile: ProviderProfile) -> umanga_core::thinking::Policy {
    umanga_core::thinking::capability(&profile).await
}
#[tauri::command]
pub async fn ollama_models(
    endpoint: String,
    force: Option<bool>,
) -> umanga_core::ollama::ModelList {
    umanga_core::ollama::list(&endpoint, force.unwrap_or(false)).await
}
#[tauri::command]
pub async fn ollama_model_details(
    endpoint: String,
    model: String,
    force: Option<bool>,
) -> umanga_core::ollama::ModelStatus {
    umanga_core::ollama::details(&endpoint, &model, force.unwrap_or(false)).await
}
#[tauri::command]
pub async fn onboarding_appearance(
    state: State<'_, AppState>,
    appearance: AppearanceSettings,
) -> Api<()> {
    let _storage = state.storage_change.lock().await;
    if !["day", "night"].contains(&appearance.active.as_str()) {
        return Err("Invalid appearance profile".into());
    }
    let mut next = state.settings.lock().clone();
    next.appearance = appearance;
    persist_preferences(&state, &next).await
}
#[tauri::command]
pub fn model_downloads(state: State<AppState>) -> Vec<String> {
    state.downloads.lock().keys().cloned().collect()
}
