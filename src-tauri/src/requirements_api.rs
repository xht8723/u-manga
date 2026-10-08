use super::*;
use umanga_core::requirements::{self, Action, Availability};

pub(super) async fn check_action(
    bundled: PathBuf,
    verification: Arc<Mutex<models::VerificationCache>>,
    settings: TranslationSettings,
    profile: Option<ProviderProfile>,
    directory: PathBuf,
    action: Action,
    cached: bool,
) -> Api<Availability> {
    let p = profile.clone();
    let s = settings.clone();
    let mut result = tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<_> {
        let key = if requirements::needs_service(action, &s) {
            p.as_ref()
                .filter(|p| providers::credential_required(p))
                .and_then(|p| credential(p).ok())
                .and_then(|k| k.get_password().ok())
                .is_some_and(|v| !v.trim().is_empty())
        } else {
            false
        };
        let mut cache = verification.lock().clone();
        requirements::check(
            action,
            &s,
            p.as_ref(),
            key,
            &models::ModelLocations {
                bundled,
                downloaded: directory,
            },
            &mut cache,
            cached,
        )
    })
    .await
    .map_err(error)?
    .map_err(error)?;
    if requirements::needs_service(action, &settings)
        && let Some(p) = profile.filter(providers::is_ollama)
    {
        let status = umanga_core::ollama::details(&p.endpoint, &p.model, false).await;
        let problem = status.message.or_else(|| {
            status.model.and_then(|m| {
                m.validate(settings.mode == "vision" && action != Action::RegionTranslate)
                    .err()
                    .map(|e| e.to_string())
            })
        });
        if let Some(message) = problem {
            result.block("ollama", "services", &message);
        }
    }
    Ok(result)
}
pub(super) async fn job_availability(
    bundled: PathBuf,
    verification: Arc<Mutex<models::VerificationCache>>,
    job: Job,
) -> Api<Availability> {
    let action = Action::from(job.kind);
    let profile = requirements::service_required(action).then_some(job.provider.clone());
    let result = check_action(
        bundled,
        verification,
        job.settings.clone(),
        profile,
        PathBuf::from(&job.model_directory),
        action,
        false,
    )
    .await?;
    Ok(result)
}
pub(super) async fn effective(state: &AppState, path: Option<String>) -> Api<AppSettings> {
    let mut settings = state.settings.lock().clone();
    if let Some(path) = path {
        let defaults = settings.clone();
        settings.translation =
            tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<_> {
                let b = umanga_core::library::open(Path::new(&path))?;
                Ok(umanga_core::library::effective(&b, &defaults))
            })
            .await
            .map_err(error)?
            .map_err(error)?;
    }
    Ok(settings)
}
pub(super) async fn applied_action(
    state: &AppState,
    settings: &AppSettings,
    action: Action,
    cached: bool,
) -> Api<Availability> {
    let profile = settings
        .providers
        .iter()
        .find(|p| p.id == settings.translation.provider_id)
        .cloned();
    check_action(
        state.bundled_models.clone(),
        state.verification.clone(),
        settings.translation.clone(),
        profile,
        settings.model_directory(),
        action,
        cached,
    )
    .await
}
pub(super) fn require(status: Availability) -> Api<()> {
    if status.ready {
        Ok(())
    } else {
        Err(status.reason().into())
    }
}

#[tauri::command]
pub async fn action_readiness(
    state: State<'_, AppState>,
    path: Option<String>,
    page: Option<Page>,
    base: Option<Page>,
    page_ids: Option<Vec<String>>,
    actions: Option<Vec<Action>>,
) -> Api<HashMap<String, Availability>> {
    let settings = effective(&state, path.clone()).await?;
    let engine = state.engine.clone();
    let lang = settings.translation.target_language.clone();
    let needs_pages = actions.as_ref().is_none_or(|a| {
        a.iter()
            .any(|a| matches!(a, Action::Cleanup | Action::Edit))
    });
    let (eligible, cached, mut edit_settings) =
        tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<_> {
            if !needs_pages {
                return Ok((false, false, None));
            }
            let eligible = if let Some(p) = &page {
                p.regions
                    .iter()
                    .any(|r| r.prepared || !r.target.trim().is_empty())
            } else if let Some(path) = path {
                store::cleanup_page_ids(Path::new(&path))?
                    .iter()
                    .any(|id| page_ids.as_ref().is_none_or(|ids| ids.contains(id)))
            } else {
                false
            };
            let cached = page
                .as_ref()
                .zip(base.as_ref())
                .is_some_and(|(p, b)| requirements::cached_edit(p, b, &engine.renderer, &lang));
            Ok((
                eligible,
                cached,
                page.and_then(|p| p.cleanup.map(|c| c.settings)),
            ))
        })
        .await
        .map_err(error)?
        .map_err(error)?;
    let mut output = HashMap::new();
    for (name, action) in [
        ("translate", Action::Translate),
        ("prepare", Action::Prepare),
        ("cleanup", Action::Cleanup),
        ("edit", Action::Edit),
        ("service", Action::Service),
        ("region_read", Action::RegionRead),
        ("region_translate", Action::RegionTranslate),
    ] {
        if actions
            .as_ref()
            .is_some_and(|wanted| !wanted.contains(&action))
        {
            continue;
        }
        let mut s = settings.clone();
        if action == Action::Edit
            && let Some(recipe) = edit_settings.take()
        {
            s.translation.cleanup = recipe;
        }
        let mut status =
            applied_action(&state, &s, action, action == Action::Edit && cached).await?;
        if action == Action::Cleanup && !eligible {
            status.block(
                "regions",
                "page",
                "Prepare or translate a page before re-cleaning.",
            );
        }
        if s.library_directory.is_empty() && action != Action::Service {
            status.block("library", "library", "Choose a library folder in Settings.");
        }
        output.insert(name.into(), status);
    }
    Ok(output)
}
#[tauri::command]
pub async fn jobs_readiness(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> Api<HashMap<String, Availability>> {
    let wanted: std::collections::HashSet<_> = ids.into_iter().collect();
    let jobs = state.engine.snapshot(Some(&wanted));
    let mut output = HashMap::new();
    let mut cache: HashMap<String, Availability> = HashMap::new();
    let engine = state.engine.clone();
    let plans = tauri::async_runtime::spawn_blocking(move || {
        engine.resume_plans(jobs.into_iter().map(|r| r.job).collect())
    })
    .await
    .map_err(error)?;
    for (job_id, plan) in plans {
        let job = match plan {
            Ok(Some(job)) => job,
            Ok(None) => {
                output.insert(job_id, Availability::default());
                continue;
            }
            Err(error) => {
                let mut status = Availability::default();
                status.block("configuration", "page", &error.to_string());
                output.insert(job_id, status);
                continue;
            }
        };
        let key =
            serde_json::to_string(&(job.kind, &job.settings, &job.provider, &job.model_directory))
                .map_err(error)?;
        let status = if let Some(status) = cache.get(&key) {
            status.clone()
        } else {
            let status = job_availability(
                state.bundled_models.clone(),
                state.verification.clone(),
                job.clone(),
            )
            .await
            .unwrap_or_else(|message| {
                let mut status = Availability::default();
                status.block("configuration", "page", &message.fallback);
                status
            });
            cache.insert(key, status.clone());
            status
        };
        output.insert(job.id, status);
    }
    Ok(output)
}
#[tauri::command]
pub async fn prepare_enqueue(
    state: State<'_, AppState>,
    path: String,
    page_id: String,
    expected: u64,
    replace: bool,
) -> Api<umanga_core::queue::Submission> {
    let _storage = state.storage_change.lock().await;
    require_managed(&state, &path)?;
    let settings = effective(&state, Some(path.clone())).await?;
    require(applied_action(&state, &settings, Action::Prepare, false).await?)?;
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || {
        engine
            .enqueue_preparation(
                &path,
                &page_id,
                expected,
                replace,
                &settings.translation,
                &settings.model_directory(),
            )
            .map(|jobs| umanga_core::queue::submission(jobs, engine.is_held()))
            .map_err(error)
    })
    .await
    .map_err(error)?
}

#[tauri::command]
pub async fn restart_enqueue(
    state: State<'_, AppState>,
    path: String,
    page_id: String,
    expected: u64,
    replace: bool,
) -> Api<umanga_core::queue::Submission> {
    let _storage = state.storage_change.lock().await;
    require_managed(&state, &path)?;
    let settings = effective(&state, Some(path.clone())).await?;
    require(applied_action(&state, &settings, Action::Translate, false).await?)?;
    let mut provider = settings
        .providers
        .iter()
        .find(|p| p.id == settings.translation.provider_id)
        .cloned()
        .ok_or("Select a translation service")?;
    provider.thinking_policy = Some(umanga_core::thinking::resolve(&provider).await);
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || {
        engine
            .enqueue_fresh(
                &path,
                &page_id,
                expected,
                replace,
                &provider,
                &settings.translation,
                &settings.model_directory(),
            )
            .map(|jobs| umanga_core::queue::submission(jobs, engine.is_held()))
            .map_err(error)
    })
    .await
    .map_err(error)?
}
