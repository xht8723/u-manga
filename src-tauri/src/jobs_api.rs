use super::*;
use serde::Serialize;
use std::{
    collections::HashSet,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
use umanga_core::{
    pipeline::ControlResult,
    queue::{LocationCache, QueueTask},
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupSubmission {
    #[serde(flatten)]
    submission: umanga_core::queue::Submission,
    untranslated: usize,
    busy: usize,
}
#[tauri::command]
pub async fn cleanup_enqueue(
    state: State<'_, AppState>,
    path: String,
    page_ids: Vec<String>,
) -> Api<CleanupSubmission> {
    let _storage = state.storage_change.lock().await;
    require_managed(&state, &path)?;
    let defaults = state.settings.lock().clone();
    let engine = state.engine.clone();
    let verification = state.verification.clone();
    tauri::async_runtime::spawn_blocking(move || -> Api<_> {
        let book = umanga_core::library::open(Path::new(&path)).map_err(error)?;
        let settings = umanga_core::library::effective(&book, &defaults);
        let translated = store::cleanup_page_ids(Path::new(&path)).map_err(error)?;
        let wanted: HashSet<_> = page_ids.into_iter().collect();
        let eligible: Vec<_> = wanted
            .iter()
            .filter(|id| translated.contains(*id))
            .cloned()
            .collect();
        // Cleanup is local: do not resolve a provider, credential, OCR pack or detector.
        if let Some(id) = settings
            .cleanup
            .method
            .pack()
            .filter(|_| !eligible.is_empty())
        {
            let pack = models::catalog()
                .map_err(error)?
                .into_iter()
                .find(|p| p.id == id)
                .ok_or("Unknown cleanup model")?;
            let mut cache = verification.lock().clone();
            if !cache
                .check(&defaults.model_directory(), &pack, false)
                .map_err(error)?
            {
                return Err(format!(
                    "Download or verify {} in Settings → Translation → Local models.",
                    pack.name
                )
                .into());
            }
        }
        let jobs = engine
            .enqueue_captured(
                &path,
                &eligible,
                &ProviderProfile::default(),
                &settings,
                JobKind::Cleanup,
                &defaults.model_directory(),
            )
            .map_err(error)?;
        let busy = eligible.len() - jobs.len();
        Ok(CleanupSubmission {
            untranslated: wanted.len() - eligible.len(),
            busy,
            submission: umanga_core::queue::submission(jobs, engine.is_held()),
        })
    })
    .await
    .map_err(error)?
}

#[derive(Default)]
pub struct JobsHub {
    dirty: Mutex<HashSet<String>>,
    force: AtomicBool,
    cache: Mutex<LocationCache>,
    recovering: AtomicBool,
    generation: AtomicU64,
    serial: AtomicU64,
    errors: Mutex<Vec<String>>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobsBatch {
    pub jobs: Vec<QueueTask>,
    pub removed: Vec<String>,
    pub held: bool,
    pub recovering: bool,
    #[serde(with = "umanga_core::ui_message::list")]
    pub errors: Vec<String>,
    pub generation: u64,
    pub version: u64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobsControl {
    pub outcome: ControlResult,
    pub state: JobsBatch,
}
impl JobsHub {
    pub fn record_error(&self, message: String) {
        self.errors.lock().push(message);
        self.pulse();
    }
    pub fn dirty(&self, id: String) {
        self.dirty.lock().insert(id);
    }
    pub fn pulse(&self) {
        self.force.store(true, Ordering::SeqCst);
    }
    pub fn invalidate(&self, path: &str, engine: &Engine) {
        self.cache.lock().invalidate(path);
        for view in engine
            .snapshot_views(None)
            .into_iter()
            .filter(|j| j.job.project == path)
        {
            self.dirty(view.job.id);
        }
        self.pulse();
    }
    fn collect(&self, engine: &Engine, ids: Option<HashSet<String>>) -> JobsBatch {
        let generation = self.generation.load(Ordering::SeqCst);
        let mut runtime = engine.snapshot_views(ids.as_ref());
        let waiting = runtime
            .iter()
            .filter(|v| !v.job.settings_captured)
            .filter_map(|v| engine.job(&v.job.id))
            .collect();
        let mut prospective = engine.resume_plans(waiting);
        for view in &mut runtime {
            if let Some(Ok(Some(job))) = prospective.remove(&view.job.id) {
                view.job = umanga_core::job_view::JobView::from_job(&job);
            }
        }
        let found: HashSet<_> = runtime.iter().map(|j| j.job.id.clone()).collect();
        let removed = ids
            .map(|ids| ids.difference(&found).cloned().collect())
            .unwrap_or_default();
        let mut cache = self.cache.lock();
        let jobs = cache.describe_views(runtime);
        let mut errors = self.errors.lock().clone();
        errors.extend(cache.errors());
        JobsBatch {
            jobs,
            removed,
            held: engine.is_held(),
            recovering: self.recovering.load(Ordering::SeqCst),
            errors,
            generation,
            version: self.serial.fetch_add(1, Ordering::SeqCst) + 1,
        }
    }
    pub fn activate(self: &Arc<Self>, root: PathBuf, engine: Arc<Engine>) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.cache.lock().clear();
        self.errors.lock().clear();
        self.dirty.lock().clear();
        self.recovering
            .store(!root.as_os_str().is_empty(), Ordering::SeqCst);
        self.pulse();
        if root.as_os_str().is_empty() {
            return;
        }
        let hub = self.clone();
        tauri::async_runtime::spawn_blocking(move || {
            match umanga_core::library::list(&root) {
                Ok(books) => {
                    for book in books {
                        if let Err(e) = engine.recover(&book.path) {
                            hub.errors
                                .lock()
                                .push(format!("{}: {e}", book.metadata.title));
                        }
                    }
                }
                Err(e) => hub.errors.lock().push(e.to_string()),
            }
            hub.recovering.store(false, Ordering::SeqCst);
            hub.pulse();
        });
    }
    pub async fn wait_recovery(&self) {
        while self.recovering.load(Ordering::SeqCst) {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }
    pub fn start(self: &Arc<Self>, app: tauri::AppHandle, engine: Arc<Engine>) {
        let hub = self.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(80)).await;
                let ids = std::mem::take(&mut *hub.dirty.lock());
                if ids.is_empty() && !hub.force.swap(false, Ordering::SeqCst) {
                    continue;
                }
                let collect = hub.clone();
                let worker = engine.clone();
                if let Ok(batch) = tauri::async_runtime::spawn_blocking(move || {
                    collect.collect(&worker, Some(ids))
                })
                .await
                    && batch.generation == hub.generation.load(Ordering::SeqCst)
                {
                    if let Some(state) = app.try_state::<AppState>() {
                        state.host.publish_jobs(&batch);
                    }
                    let _ = app.emit("jobs", batch);
                }
            }
        });
    }
}
#[tauri::command]
pub async fn jobs_list(state: State<'_, AppState>) -> Api<JobsBatch> {
    let hub = state.jobs_hub.clone();
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || hub.collect(&engine, None))
        .await
        .map_err(error)
}
#[tauri::command]
pub async fn job_control(
    state: State<'_, AppState>,
    id: String,
    action: umanga_core::job_state::JobAction,
) -> Api<JobsControl> {
    let intent = state.engine.request_job_control(&id).map_err(error)?;
    let action = action.as_str().to_owned();
    if ["resume", "retry"].contains(&action.as_str()) {
        let original = state.engine.job(&id).ok_or("Job not found")?;
        let engine = state.engine.clone();
        let job = tauri::async_runtime::spawn_blocking(move || engine.prospective_job(original))
            .await
            .map_err(error)?
            .map_err(error)?;
        if let Err(e) = if state.engine.job_omitted(&job).map_err(error)? {
            Ok(())
        } else {
            state.engine.check_requirements(job).await
        } {
            let engine = state.engine.clone();
            let key = id.clone();
            let reason = e.to_string();
            tauri::async_runtime::spawn_blocking(move || {
                engine.block_job_requested(&key, &reason, Some(intent))
            })
            .await
            .map_err(error)?;
            return Err(error(e));
        }
    }
    let hub = state.jobs_hub.clone();
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let outcome = match engine.control_requested(&id, &action, intent) {
            Ok(()) => ControlResult {
                changed: 1,
                ..Default::default()
            },
            Err(e) => ControlResult {
                errors: vec![umanga_core::pipeline::JobFailure {
                    id: id.clone(),
                    message: e.to_string(),
                }],
                ..Default::default()
            },
        };
        JobsControl {
            outcome,
            state: hub.collect(&engine, Some(HashSet::from([id]))),
        }
    })
    .await
    .map_err(error)
}
#[tauri::command]
pub async fn jobs_control_all(state: State<'_, AppState>, action: String) -> Api<JobsControl> {
    let request = state.engine.request_control(&action).map_err(error)?;
    state.jobs_hub.pulse();
    if action == "start" {
        state.jobs_hub.wait_recovery().await;
    }
    let mut blocked = HashMap::new();
    if action == "start" {
        let ids = state
            .engine
            .snapshot_views(None)
            .into_iter()
            .map(|view| view.job)
            .filter(|j| ["queued", "paused", "failed"].contains(&j.status.as_str()))
            .map(|j| j.id)
            .collect();
        for (id, status) in jobs_readiness(state.clone(), ids).await? {
            if !status.ready {
                blocked.insert(id, status.reason());
            }
        }
    }
    let hub = state.jobs_hub.clone();
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || -> Api<_> {
        let outcome = engine
            .control_all_ready(&action, request, &blocked)
            .map_err(error)?;
        hub.pulse();
        Ok(JobsControl {
            outcome,
            state: hub.collect(&engine, None),
        })
    })
    .await
    .map_err(error)?
}
