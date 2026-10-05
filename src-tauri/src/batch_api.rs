//! Book/chapter options are transient; durable replacement intent lives in existing fresh jobs.
use crate::{Api, AppState, error, require_managed, requirements_api};
use serde::Serialize;
use tauri::State;
use umanga_core::{
    pipeline::{BatchMode, BatchOutcome, BatchPreview, PageVersion},
    queue,
    requirements::Action,
    thinking,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchReply {
    jobs: Vec<queue::QueueTask>,
    warnings: Vec<umanga_core::pipeline::JobFailure>,
    #[serde(flatten)]
    outcome: BatchOutcome,
}

#[tauri::command]
pub async fn translation_batch_preview(
    state: State<'_, AppState>,
    path: String,
    chapter_id: Option<String>,
) -> Api<BatchPreview> {
    let _storage = state.storage_change.lock().await;
    require_managed(&state, &path)?;
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || engine.preview_batch(&path, chapter_id.as_deref()))
        .await
        .map_err(error)?
        .map_err(error)
}

#[tauri::command]
pub async fn translation_batch_submit(
    state: State<'_, AppState>,
    path: String,
    book_id: String,
    chapter_id: Option<String>,
    pages: Vec<PageVersion>,
    mode: BatchMode,
) -> Api<BatchReply> {
    let _storage = state.storage_change.lock().await;
    require_managed(&state, &path)?;
    let settings = requirements_api::effective(&state, Some(path.clone())).await?;
    let readiness =
        requirements_api::applied_action(&state, &settings, Action::Translate, false).await?;
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
    profile.thinking_policy = Some(thinking::resolve(&profile).await);
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || {
        engine
            .enqueue_batch(
                &path,
                &book_id,
                chapter_id.as_deref(),
                &pages,
                mode,
                &profile,
                &settings.translation,
                &settings.model_directory(),
            )
            .map(|result| {
                let submitted = queue::submission(result.jobs, engine.is_held());
                BatchReply {
                    jobs: submitted.jobs,
                    warnings: submitted.warnings,
                    outcome: result.outcome,
                }
            })
    })
    .await
    .map_err(error)?
    .map_err(error)
}
