use super::*;
use umanga_core::{
    pipeline::{RegionAction, RegionRequest, RegionResult},
    requirements::Action,
};

async fn run(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: RegionRequest,
    action: RegionAction,
) -> Api<RegionResult> {
    let storage = state.storage_change.lock().await;
    require_managed(&state, &request.path)?;
    let settings = requirements_api::effective(&state, Some(request.path.clone())).await?;
    let engine = state.engine.clone();
    let input = request.clone();
    let guard = tauri::async_runtime::spawn_blocking(move || {
        engine.reserve_region(&input.id, &input.path, &input.page_id)
    })
    .await
    .map_err(error)?
    .map_err(error)?;
    drop(storage);
    // Reserve before asynchronous readiness so Stop also cancels validation-in-flight requests.
    let id = request.id.clone();
    let started = std::time::Instant::now();
    let progress: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(move |stage| {
        let _ = app.emit("region-progress", serde_json::json!({"id": id, "stage": stage, "elapsedMs": started.elapsed().as_millis() as u64}));
    });
    progress("checking");
    let operation = if action == RegionAction::Read {
        Action::RegionRead
    } else {
        Action::RegionTranslate
    };
    let token = guard.cancel.clone();
    let ready = tokio::select! { _ = token.cancelled() => return Err("Cancelled".into()), result = requirements_api::applied_action(&state, &settings, operation, false) => result? };
    requirements_api::require(ready)?;
    let mut provider = settings
        .providers
        .iter()
        .find(|p| p.id == settings.translation.provider_id)
        .cloned()
        .unwrap_or_default();
    if umanga_core::requirements::needs_service(operation, &settings.translation) {
        provider.thinking_policy = Some(
            tokio::select! { _ = token.cancelled() => return Err("Cancelled".into()), result = umanga_core::thinking::resolve(&provider) => result },
        );
    }
    let mut result = state
        .engine
        .process_region(
            request,
            action,
            settings.translation.clone(),
            provider,
            settings.model_directory(),
            guard,
            progress,
        )
        .await
        .map_err(error)?;
    result.elapsed_ms = started.elapsed().as_millis() as u64;
    Ok(result)
}
#[tauri::command]
pub async fn region_read(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: RegionRequest,
) -> Api<RegionResult> {
    run(app, state, request, RegionAction::Read).await
}
#[tauri::command]
pub async fn region_translate(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: RegionRequest,
) -> Api<RegionResult> {
    run(app, state, request, RegionAction::Translate).await
}
#[tauri::command]
pub fn region_cancel(state: State<'_, AppState>, id: String) {
    state.engine.cancel_region(&id);
}
