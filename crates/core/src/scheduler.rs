// Scheduling and durable worker state. State locks protect memory only. The separate operation
// gate orders durable transitions on background threads; it never blocks the UI.
use super::*;
use serde::Serialize;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

pub(super) struct Entry {
    pub(super) job: Job,
    pub(super) requirements_key: String,
    pub(super) token: CancellationToken,
    pub(super) sequence: u64,
    pub(super) page_order: usize,
    pub(super) clock: Option<(Instant, u64)>,
}
impl Entry {
    pub(super) fn elapsed(&self) -> u64 {
        self.clock
            .map(|(start, base)| base.saturating_add(start.elapsed().as_millis() as u64))
            .unwrap_or(self.job.elapsed_ms)
    }
    pub(super) fn sampled(&self) -> Job {
        let mut job = self.job.clone();
        job.elapsed_ms = self.elapsed();
        job
    }
}
pub type JobCheck =
    Arc<dyn Fn(Job) -> futures_util::future::BoxFuture<'static, Result<()>> + Send + Sync>;

pub(super) struct CapturedSubmission<'a> {
    pub path: &'a str,
    pub provider: &'a ProviderProfile,
    pub settings: &'a TranslationSettings,
    pub kind: JobKind,
    pub directory: &'a Path,
}

#[cfg(test)]
mod scheduler_contracts {
    use super::*;
    #[test]
    fn stop_during_submission_keeps_durable_jobs_visible_after_a_pause_write_failure() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("original.png");
        image::RgbImage::new(64, 64).save(&source).unwrap();
        let pages = documents::import(&[source.to_string_lossy().into()]).unwrap();
        let book = store::create(&tmp.path().join("book.umanga"), "Submission", &pages).unwrap();
        let engine = Engine::new(
            root.join("assets/models"),
            &root.join("assets/fonts"),
            Arc::new(|_| {}),
            Arc::new(|_| bail!("No service calls")),
        )
        .unwrap();
        let provider = ProviderProfile::default();
        let settings = TranslationSettings::default();
        let captured = CapturedSubmission {
            path: &book.path,
            provider: &provider,
            settings: &settings,
            kind: JobKind::Translation,
            directory: tmp.path(),
        };
        let job = engine.captured_job(&captured, &pages[0].id, None);
        let positions = HashMap::from([(pages[0].id.clone(), 0)]);
        let submitted = engine.publish_enqueued_at_commit(&book.path, vec![job.clone()], &positions, || {
            engine.request_control("stop").unwrap();
            store::connection(Path::new(&book.path)).unwrap().execute_batch("CREATE TRIGGER fail_pause BEFORE INSERT ON jobs WHEN json_extract(NEW.data,'$.status')='paused' BEGIN SELECT RAISE(ABORT,'injected pause failure'); END;").unwrap();
        });
        assert!(
            submitted.is_ok(),
            "the first transaction committed: {submitted:?}"
        );
        let visible = engine.list();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, job.id);
        assert_eq!(visible[0].status, "paused");
        assert!(visible[0].error.is_some());
        assert!(engine.pick().is_none());
        assert_eq!(store::jobs(Path::new(&book.path)).unwrap()[0].id, job.id);
    }
    #[test]
    fn checkpoints_publish_committed_boxes_and_ocr_and_reject_stale_or_cancelled_writes() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let tmp = tempfile::tempdir_in(root.join("test-output")).unwrap();
        let source = tmp.path().join("original.png");
        image::RgbImage::new(64, 64).save(&source).unwrap();
        let pages = documents::import(&[source.to_string_lossy().into()]).unwrap();
        let book = store::create(&tmp.path().join("book.umanga"), "Progress", &pages).unwrap();
        let observed = Arc::new(std::sync::Mutex::new(Vec::<u64>::new()));
        let events = observed.clone();
        let engine = Engine::new(
            root.join("assets/models"),
            &root.join("assets/fonts"),
            Arc::new(move |job| {
                if let Some(revision) = job.page_revision {
                    let saved = store::page(Path::new(&job.project), &job.page_id).unwrap();
                    assert!(
                        saved.revision >= revision,
                        "Events must follow durable writes"
                    );
                    events.lock().unwrap().push(revision);
                }
            }),
            Arc::new(|_| bail!("No service calls")),
        )
        .unwrap();
        engine
            .enqueue(
                &book.path,
                &[pages[0].id.clone()],
                &ProviderProfile::default(),
            )
            .unwrap();
        let (job, token) = engine.pick().unwrap();
        let mut p = pages[0].clone();
        let mut revision = 0;
        engine.progress(&job.id, "detecting", "");
        p.regions = vec![Region {
            bbox: [1., 1., 63., 63.],
            ..Default::default()
        }];
        engine
            .checkpoint(
                &job.id,
                &mut p,
                &mut revision,
                "detecting",
                "complete",
                "1 region saved",
            )
            .unwrap();
        engine.progress(&job.id, "ocr", "");
        p.regions[0].source = "日本語".into();
        engine
            .checkpoint(
                &job.id,
                &mut p,
                &mut revision,
                "ocr",
                "complete",
                "Text saved",
            )
            .unwrap();
        let mut manual = p.clone();
        manual.regions[0].target = "manual".into();
        store::save_page(Path::new(&book.path), &mut manual, revision).unwrap();
        p.regions[0].target = "late".into();
        assert!(
            engine
                .checkpoint(
                    &job.id,
                    &mut p,
                    &mut revision,
                    "translating",
                    "complete",
                    ""
                )
                .is_err()
        );
        engine.control(&job.id, "pause").unwrap();
        assert!(
            engine
                .checkpoint(&job.id, &mut p, &mut revision, "saving", "complete", "")
                .is_err()
        );
        engine.finish(&job.id, &token, Ok(()), 1);
        let saved = store::page(Path::new(&book.path), &p.id).unwrap();
        assert_eq!(saved.regions[0].source, "日本語");
        assert_eq!(saved.regions[0].target, "manual");
        assert!(observed.lock().unwrap().contains(&2));
        let saved_job = store::jobs(Path::new(&book.path)).unwrap().remove(0);
        assert!(
            saved_job
                .steps
                .iter()
                .any(|s| s.stage == "detecting" && s.status == "complete")
        );
        assert!(
            saved_job
                .steps
                .iter()
                .any(|s| s.stage == "ocr" && s.status == "complete")
        );
    }
    #[test]
    fn stopping_each_stage_keeps_one_attempt_and_ignores_late_progress() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let tmp = tempfile::tempdir_in(root.join("test-output")).unwrap();
        let source = tmp.path().join("page.png");
        image::RgbImage::from_pixel(8, 8, image::Rgb([255, 255, 255]))
            .save(&source)
            .unwrap();
        let base = documents::import(&[source.to_string_lossy().into()])
            .unwrap()
            .remove(0);
        let stages = [
            "loading",
            "detecting",
            "ocr",
            "translating",
            "rendering",
            "saving",
        ];
        let pages: Vec<_> = (0..stages.len())
            .map(|i| {
                let mut p = base.clone();
                p.id = uid();
                p.number = i;
                p
            })
            .collect();
        let book = store::create(&tmp.path().join("book.umanga"), "Stages", &pages).unwrap();
        let engine = Engine::new(
            root.join("assets/models"),
            &root.join("assets/fonts"),
            Arc::new(|_| {}),
            Arc::new(|_| bail!("No provider calls")),
        )
        .unwrap();
        for (stage, page) in stages.into_iter().zip(pages) {
            engine
                .enqueue(&book.path, &[page.id], &ProviderProfile::default())
                .unwrap();
            let (job, token) = engine.pick().unwrap();
            engine.progress(&job.id, stage, "stage boundary");
            engine.control_all("stop").unwrap();
            assert!(
                engine
                    .snapshot(None)
                    .iter()
                    .any(|j| j.job.id == job.id && j.stopping)
            );
            engine.control_all("start").unwrap();
            assert!(
                engine.pick().is_none(),
                "Old attempt must exit before restarting"
            );
            engine.progress(&job.id, "saving", "late");
            assert_eq!(
                engine.list().iter().find(|j| j.id == job.id).unwrap().stage,
                stage
            );
            engine.finish(&job.id, &token, Ok(()), 10);
            let (retry, new_token) = engine.pick().unwrap();
            assert_eq!(retry.id, job.id);
            assert!(!new_token.is_cancelled());
            engine.finish(&retry.id, &new_token, Ok(()), 20);
            assert_eq!(
                engine
                    .list()
                    .iter()
                    .filter(|j| j.page_id == retry.page_id)
                    .count(),
                1
            );
        }
        assert!(engine.list().iter().all(|j| j.status == "complete"));
    }
}
#[derive(Default)]
pub(super) struct Scheduler {
    pub(super) jobs: HashMap<String, Entry>,
    control_intents: HashMap<String, u64>,
    bulk_intents: HashMap<String, u64>,
    pub(super) pending: VecDeque<String>,
    pub(super) active: HashSet<String>,
    pub(super) blocked: HashSet<String>,
    pub(super) editing: HashSet<(String, String)>,
    pub(super) regions: HashMap<String, RegionOperation>,
    pub(super) recovered: HashSet<String>,
    pub(super) restart: HashSet<String>,
    pub(super) ongoing: HashMap<(String, String), String>,
    pub(super) latest: HashMap<(String, String), String>,
    pub(super) revision: u64,
    dispatch_sequence: u64,
    last_dispatch: HashMap<String, u64>,
}
impl Scheduler {
    fn busy_book(&self, path: &str) -> bool {
        self.editing.iter().any(|(p, _)| p == path)
            || self
                .active
                .iter()
                .any(|id| self.jobs[id].job.project == path)
    }
    fn active_books(&self) -> usize {
        self.editing
            .iter()
            .map(|(p, _)| p.as_str())
            .chain(
                self.active
                    .iter()
                    .map(|id| self.jobs[id].job.project.as_str()),
            )
            .collect::<HashSet<_>>()
            .len()
    }
    pub(super) fn touch(&mut self, id: &str) -> Job {
        self.revision += 1;
        let entry = self.jobs.get_mut(id).unwrap();
        entry.job.elapsed_ms = entry.elapsed();
        entry.sequence = self.revision;
        let key = (entry.job.project.clone(), entry.job.page_id.clone());
        if self.active.contains(id)
            || ["running", "queued", "paused"].contains(&entry.job.status.as_str())
        {
            self.ongoing.insert(key, id.into());
        } else if self.ongoing.get(&key).is_some_and(|v| v == id) {
            self.ongoing.remove(&key);
        }
        entry.job.clone()
    }
    pub(super) fn reindex(&mut self) {
        self.ongoing.clear();
        self.latest.clear();
        let mut entries: Vec<_> = self.jobs.values().collect();
        entries.sort_by(|a, b| {
            a.job
                .created
                .cmp(&b.job.created)
                .then(a.sequence.cmp(&b.sequence))
        });
        for e in entries {
            let key = (e.job.project.clone(), e.job.page_id.clone());
            self.latest.insert(key.clone(), e.job.id.clone());
            if self.active.contains(&e.job.id)
                || ["running", "queued", "paused"].contains(&e.job.status.as_str())
            {
                self.ongoing.insert(key, e.job.id.clone());
            }
        }
    }
    pub(super) fn order_pending(&mut self) {
        let mut ids: Vec<_> = self.pending.drain(..).collect();
        let books: HashSet<_> = ids
            .iter()
            .map(|id| self.jobs[id].job.project.clone())
            .collect();
        for book in books {
            let positions: Vec<_> = ids
                .iter()
                .enumerate()
                .filter(|(_, id)| self.jobs[*id].job.project == book)
                .map(|(i, _)| i)
                .collect();
            let mut ordered: Vec<_> = positions.iter().map(|i| ids[*i].clone()).collect();
            ordered.sort_by_key(|id| self.jobs[id].page_order);
            for (position, id) in positions.into_iter().zip(ordered) {
                ids[position] = id;
            }
        }
        self.pending = ids.into();
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeJob {
    #[serde(flatten)]
    pub job: Job,
    pub sequence: u64,
    pub stopping: bool,
    pub timer_running: bool,
}
pub struct PageEditGuard {
    engine: Arc<Engine>,
    key: (String, String),
}
pub struct RegionGuard {
    engine: Arc<Engine>,
    id: String,
    pub cancel: CancellationToken,
}
pub(super) struct RegionOperation {
    book: String,
    page: String,
    cancel: CancellationToken,
}
impl Drop for RegionGuard {
    fn drop(&mut self) {
        let mut state = self.engine.state.lock();
        if let Some(operation) = state.regions.remove(&self.id) {
            state.editing.remove(&(operation.book, operation.page));
        }
    }
}
pub struct BookReservation {
    engine: Arc<Engine>,
    path: String,
}
impl Drop for BookReservation {
    fn drop(&mut self) {
        if !self.path.is_empty() {
            self.engine.state.lock().blocked.remove(&self.path);
        }
    }
}
impl Drop for PageEditGuard {
    fn drop(&mut self) {
        self.engine.state.lock().editing.remove(&self.key);
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobFailure {
    pub id: String,
    #[serde(with = "crate::ui_message::text")]
    pub message: String,
}
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlResult {
    pub changed: usize,
    pub scheduled: usize,
    pub restarting: usize,
    pub omitted: usize,
    pub blocked: usize,
    pub skipped: usize,
    pub errors: Vec<JobFailure>,
}
pub struct Engine {
    pub inference: Mutex<Inference>,
    pub renderer: Renderer,
    pub inpainting: crate::inpainting::Worker,
    pub model_directory: Mutex<PathBuf>,
    pub(super) state: Mutex<Scheduler>,
    pub(super) operations: Mutex<()>,
    pub(super) held: AtomicBool,
    concurrent_books: AtomicUsize,
    pub(super) application_settings: Mutex<AppSettings>,
    pub(super) application_configured: AtomicBool,
    pub(super) control_serial: AtomicU64,
    pub(super) notify: Notify,
    pub(super) secret: Secret,
    pub(super) requirements: Mutex<Option<JobCheck>>,
}
impl Engine {
    pub(super) fn inference_for(
        &self,
        cancel: &CancellationToken,
    ) -> Result<parking_lot::MutexGuard<'_, Inference>> {
        loop {
            anyhow::ensure!(!cancel.is_cancelled(), "Cancelled");
            if let Some(guard) = self
                .inference
                .try_lock_for(std::time::Duration::from_millis(20))
            {
                return Ok(guard);
            }
        }
    }
    pub fn new(models: PathBuf, fonts: &Path, notify: Notify, secret: Secret) -> Result<Arc<Self>> {
        Self::with_locations(
            crate::models::ModelLocations {
                bundled: models.clone(),
                downloaded: models,
            },
            fonts,
            notify,
            secret,
        )
    }
    pub fn with_locations(
        locations: crate::models::ModelLocations,
        fonts: &Path,
        notify: Notify,
        secret: Secret,
    ) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            model_directory: Mutex::new(locations.downloaded.clone()),
            inpainting: crate::inpainting::Worker::new(),
            inference: Mutex::new(Inference::with_locations(locations)),
            renderer: Renderer::new(fonts)?,
            state: Mutex::new(Scheduler::default()),
            operations: Mutex::new(()),
            held: AtomicBool::new(false),
            concurrent_books: AtomicUsize::new(2),
            application_settings: Mutex::new(AppSettings::default()),
            application_configured: AtomicBool::new(false),
            control_serial: AtomicU64::new(0),
            notify,
            secret,
            requirements: Mutex::new(None),
        }))
    }
    pub fn configure(&self, settings: &AppSettings) {
        self.concurrent_books
            .store(settings.concurrent_books.clamp(1, 4), Ordering::SeqCst);
        *self.application_settings.lock() = settings.clone();
        self.application_configured.store(true, Ordering::SeqCst);
        let pending: Vec<_> = self
            .state
            .lock()
            .jobs
            .values()
            .filter(|e| {
                !e.job.settings_captured && ["queued", "paused"].contains(&e.job.status.as_str())
            })
            .map(|e| e.job.clone())
            .collect();
        for job in pending {
            (self.notify)(job);
        }
    }
    pub fn is_held(&self) -> bool {
        self.held.load(Ordering::SeqCst)
    }
    pub fn reserve_region(
        self: &Arc<Self>,
        id: &str,
        path: &str,
        page: &str,
    ) -> Result<RegionGuard> {
        crate::safety::identity(id)?;
        let _op = self.operations.lock();
        let mut state = self.state.lock();
        anyhow::ensure!(!self.is_held(), "Use Start all in Jobs first.");
        let key = (path.to_owned(), page.to_owned());
        anyhow::ensure!(
            !state.blocked.contains(path),
            "Book organization is being applied"
        );
        anyhow::ensure!(
            !state.editing.contains(&key) && !state.ongoing.contains_key(&key),
            "This page already has an operation; wait for it to finish"
        );
        anyhow::ensure!(!state.regions.contains_key(id), "Duplicate region request");
        anyhow::ensure!(
            !state.busy_book(path),
            "This book is processing another page; wait for it to finish"
        );
        anyhow::ensure!(
            state.active_books() < self.concurrent_books.load(Ordering::SeqCst),
            "Processing limit reached; wait for an active book to finish"
        );
        let cancel = CancellationToken::new();
        state.editing.insert(key);
        state.regions.insert(
            id.into(),
            RegionOperation {
                book: path.into(),
                page: page.into(),
                cancel: cancel.clone(),
            },
        );
        Ok(RegionGuard {
            engine: self.clone(),
            id: id.into(),
            cancel,
        })
    }
    pub fn cancel_region(&self, id: &str) {
        if let Some(operation) = self.state.lock().regions.get(id) {
            operation.cancel.cancel();
        }
    }
    pub fn set_requirements(&self, check: JobCheck) {
        *self.requirements.lock() = Some(check);
    }
    pub async fn check_requirements(&self, job: Job) -> Result<()> {
        let check = self.requirements.lock().clone();
        if let Some(check) = check {
            check(job).await?;
        }
        Ok(())
    }
    pub fn block_job(&self, id: &str, reason: &str) {
        self.block_job_requested(id, reason, None);
    }
    pub fn block_job_requested(&self, id: &str, reason: &str, intent: Option<(u64, u64)>) {
        let _op = self.operations.lock();
        let mut state = self.state.lock();
        if intent.is_some_and(|(generation, global)| {
            state.control_intents.get(id) != Some(&generation)
                || self.control_serial.load(Ordering::SeqCst) != global
        }) {
            return;
        }
        if intent.is_some() && state.active.contains(id) {
            return;
        }
        let Some(e) = state.jobs.get_mut(id) else {
            return;
        };
        if ["complete", "cancelled"].contains(&e.job.status.as_str()) {
            return;
        }
        e.token.cancel();
        e.job.status = "paused".into();
        e.job.error = Some(reason.into());
        e.job.failure_kind = Some(crate::job_state::FailureKind::Requirements);
        e.job.stage_detail = "Requirements unavailable".into();
        state.pending.retain(|v| v != id);
        let job = state.touch(id);
        drop(state);
        self.persist_notify(vec![job]);
    }
    /// Immediate dispatch gate and cancellation, independent of slow storage.
    pub fn hold(&self) {
        self.held.store(true, Ordering::SeqCst);
        let state = self.state.lock();
        for operation in state.regions.values() {
            operation.cancel.cancel();
        }
        for e in state.jobs.values() {
            if ["queued", "running"].contains(&e.job.status.as_str()) {
                e.token.cancel();
            }
        }
        let active: Vec<_> = state
            .active
            .iter()
            .filter_map(|id| state.jobs.get(id))
            .map(|e| e.job.clone())
            .collect();
        drop(state);
        for job in active {
            (self.notify)(job);
        }
    }
    pub fn revision(&self) -> u64 {
        self.state.lock().revision
    }
    pub fn snapshot(&self, ids: Option<&HashSet<String>>) -> Vec<RuntimeJob> {
        let state = self.state.lock();
        let entries: Vec<_> = match ids {
            Some(ids) => ids.iter().filter_map(|id| state.jobs.get(id)).collect(),
            None => state.jobs.values().collect(),
        };
        let mut jobs: Vec<_> = entries
            .into_iter()
            .map(|e| RuntimeJob {
                job: e.sampled(),
                sequence: e.sequence,
                stopping: state.active.contains(&e.job.id) && e.token.is_cancelled(),
                timer_running: e.clock.is_some(),
            })
            .collect();
        drop(state);
        jobs.sort_by(|a, b| {
            a.job
                .created
                .cmp(&b.job.created)
                .then(a.sequence.cmp(&b.sequence))
        });
        jobs
    }
    pub fn list(&self) -> Vec<Job> {
        self.snapshot(None).into_iter().map(|j| j.job).collect()
    }
    pub fn snapshot_views(
        &self,
        ids: Option<&HashSet<String>>,
    ) -> Vec<crate::job_view::RuntimeView> {
        let state = self.state.lock();
        let entries: Vec<_> = match ids {
            Some(ids) => ids.iter().filter_map(|id| state.jobs.get(id)).collect(),
            None => state.jobs.values().collect(),
        };
        let mut views: Vec<_> = entries
            .into_iter()
            .map(|entry| {
                let mut job = crate::job_view::JobView::with_requirements_key(
                    &entry.job,
                    entry.requirements_key.clone(),
                );
                job.elapsed_ms = entry.elapsed();
                crate::job_view::RuntimeView {
                    job,
                    sequence: entry.sequence,
                    stopping: state.active.contains(&entry.job.id) && entry.token.is_cancelled(),
                    timer_running: entry.clock.is_some(),
                }
            })
            .collect();
        drop(state);
        views.sort_by(|a, b| {
            a.job
                .created
                .cmp(&b.job.created)
                .then(a.sequence.cmp(&b.sequence))
        });
        views
    }
    // Caller holds operations, never state. All writes for a book use one transaction.
    pub(super) fn persist_notify(&self, jobs: Vec<Job>) -> Vec<JobFailure> {
        let mut groups: HashMap<String, Vec<Job>> = HashMap::new();
        for job in jobs {
            groups.entry(job.project.clone()).or_default().push(job);
        }
        let mut errors = vec![];
        for (path, mut jobs) in groups {
            if let Err(error) = store::save_jobs(Path::new(&path), &jobs) {
                let mut state = self.state.lock();
                for job in &mut jobs {
                    let message = format!("Could not save job: {error}");
                    errors.push(JobFailure {
                        id: job.id.clone(),
                        message: message.clone(),
                    });
                    if let Some(e) = state.jobs.get_mut(&job.id) {
                        e.token.cancel();
                        if ["queued", "running"].contains(&e.job.status.as_str()) {
                            e.job.status = "paused".into();
                        }
                        e.job.error = Some(message);
                        e.job.failure_kind = Some(crate::job_state::FailureKind::Storage);
                        *job = state.touch(&job.id);
                    }
                }
            }
            for job in jobs {
                (self.notify)(job);
            }
        }
        errors
    }
    pub fn start(self: &Arc<Self>) {
        let clock = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                let engine = clock.clone();
                let _ = tokio::task::spawn_blocking(move || engine.timer_checkpoint()).await;
            }
        });
        let engine = self.clone();
        tokio::spawn(async move {
            loop {
                let picker = engine.clone();
                let picked = tokio::task::spawn_blocking(move || picker.pick())
                    .await
                    .ok()
                    .flatten();
                if let Some(snapshot) = picked {
                    let worker = engine.clone();
                    let handle = tokio::runtime::Handle::current();
                    tokio::task::spawn_blocking(move || {
                        let start = Instant::now();
                        let id = snapshot.0.id.clone();
                        let token = snapshot.1.clone();
                        let elapsed = snapshot.0.elapsed_ms;
                        let result = handle.block_on(worker.clone().process(&id, snapshot));
                        worker.finish(
                            &id,
                            &token,
                            result,
                            elapsed + start.elapsed().as_millis() as u64,
                        );
                    });
                }
                tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            }
        });
    }
    pub(super) fn timer_checkpoint(&self) {
        let Some(_op) = self.operations.try_lock() else {
            return;
        };
        let mut state = self.state.lock();
        let ids: Vec<_> = state
            .active
            .iter()
            .filter(|id| state.jobs.contains_key(*id))
            .cloned()
            .collect();
        let jobs = ids.iter().map(|id| state.touch(id)).collect();
        drop(state);
        self.persist_notify(jobs);
    }
    pub(super) fn pick(&self) -> Option<(Job, CancellationToken)> {
        if self.is_held() {
            return None;
        }
        let _op = self.operations.try_lock()?;
        if self.is_held() {
            return None;
        }
        let mut state = self.state.lock();
        if state.active_books() >= self.concurrent_books.load(Ordering::SeqCst) {
            return None;
        }
        let eligible = |id: &&String| {
            state.jobs.get(*id).is_some_and(|e| {
                e.job.status == "queued"
                    && !state.blocked.contains(&e.job.project)
                    && !state.busy_book(&e.job.project)
            })
        };
        let index = state
            .pending
            .iter()
            .enumerate()
            .filter(|(_, id)| eligible(id))
            .min_by_key(|(i, id)| {
                (
                    state
                        .last_dispatch
                        .get(&state.jobs[*id].job.project)
                        .copied()
                        .unwrap_or(0),
                    *i,
                )
            })
            .map(|(i, _)| i)?;
        let id = state.pending.remove(index)?;
        let book = state.jobs[&id].job.project.clone();
        state.dispatch_sequence += 1;
        let sequence = state.dispatch_sequence;
        state.last_dispatch.insert(book, sequence);
        state.active.insert(id.clone());
        let e = state.jobs.get_mut(&id)?;
        e.job.status = "running".into();
        e.job.stage = "starting".into();
        e.job.stage_detail.clear();
        e.clock = Some((Instant::now(), e.job.elapsed_ms));
        let token = e.token.clone();
        let job = state.touch(&id);
        drop(state);
        if !self.persist_notify(vec![job.clone()]).is_empty() {
            let mut state = self.state.lock();
            state.active.remove(&id);
            if let Some(e) = state.jobs.get_mut(&id) {
                e.job.elapsed_ms = e.elapsed();
                e.clock = None;
            }
            return None;
        }
        Some((job, token))
    }
    pub(super) fn finish(
        &self,
        id: &str,
        token: &CancellationToken,
        result: Result<()>,
        elapsed: u64,
    ) {
        let _op = self.operations.lock();
        let mut state = self.state.lock();
        let restart = state.restart.remove(id) && !self.is_held();
        let Some(e) = state.jobs.get_mut(id) else {
            state.active.remove(id);
            return;
        };
        e.job.elapsed_ms = e.elapsed().max(elapsed);
        e.clock = None;
        if restart && e.job.status != "cancelled" {
            e.token = CancellationToken::new();
            e.job.status = "queued".into();
            e.job.stage = "waiting".into();
            e.job.stage_detail.clear();
            e.job.error = None;
            if !state.pending.iter().any(|v| v == id) {
                state.pending.push_back(id.into());
            }
        } else if token.is_cancelled() {
            if e.job.status == "running" {
                e.job.status = "paused".into();
            }
        } else {
            match result {
                Ok(()) => {
                    for step in &mut e.job.steps {
                        if step.status == "running" {
                            step.status = "complete".into();
                        }
                    }
                    e.job.status = "complete".into();
                    e.job.stage = "done".into();
                    e.job.stage_detail.clear();
                    e.job.error = None;
                }
                Err(error) => {
                    e.job.status = "failed".into();
                    e.job.failure_kind = Some(
                        error
                            .downcast_ref::<crate::job_state::ExecutionError>()
                            .map_or(crate::job_state::FailureKind::Processing, |e| e.kind),
                    );
                    if error.downcast_ref::<rusqlite::Error>().is_some()
                        || error.downcast_ref::<std::io::Error>().is_some()
                    {
                        e.job.failure_kind = Some(crate::job_state::FailureKind::Storage);
                    }
                    if matches!(
                        e.job.failure_kind,
                        Some(
                            crate::job_state::FailureKind::Configuration
                                | crate::job_state::FailureKind::Requirements
                        )
                    ) {
                        e.job.status = "paused".into();
                    }
                    let message = format!("{error:#}");
                    e.job.record_step(
                        &e.job.stage.clone(),
                        "failed",
                        &e.job.stage_detail.clone(),
                        Some(message.clone()),
                    );
                    e.job.error = Some(message);
                }
            }
        }
        let job = state.touch(id);
        state.order_pending();
        drop(state);
        self.persist_notify(vec![job.clone()]);
        // Organization must wait for the worker's final write, not just computation.
        {
            let mut state = self.state.lock();
            state.active.remove(id);
            state.touch(id);
        }
        (self.notify)(job);
    }
    pub fn cleanup_progress(
        &self,
        id: &str,
        stage: &str,
        device: &str,
        detail: &str,
        model: CleanupMethod,
    ) {
        let mut state = self.state.lock();
        let Some(e) = state.jobs.get_mut(id) else {
            return;
        };
        if e.job.status != "running" || e.token.is_cancelled() {
            return;
        }
        e.job.begin_step(stage, detail);
        e.job.device = device.into();
        e.job.cleanup_model = Some(model);
        let job = state.touch(id);
        drop(state);
        (self.notify)(job);
    }
    pub(super) fn progress(&self, id: &str, stage: &str, detail: &str) {
        let _op = self.operations.lock();
        let mut state = self.state.lock();
        let Some(e) = state.jobs.get_mut(id) else {
            return;
        };
        if e.job.status != "running" || e.token.is_cancelled() {
            return;
        }
        e.job.begin_step(stage, detail);
        if stage == "ocr" && !detail.is_empty() {
            e.job.device = detail.into();
        }
        let job = state.touch(id);
        drop(state);
        self.persist_notify(vec![job]);
    }
    pub(super) fn step_result(&self, id: &str, stage: &str, status: &str, detail: &str) {
        let _op = self.operations.lock();
        let mut state = self.state.lock();
        let Some(e) = state.jobs.get_mut(id) else {
            return;
        };
        if e.job.status != "running" || e.token.is_cancelled() {
            return;
        }
        e.job.record_step(stage, status, detail, None);
        let job = state.touch(id);
        drop(state);
        self.persist_notify(vec![job]);
    }
    // Commit the page before publishing its revision. The operation gate orders
    // this with Pause/Cancel; the scheduler state lock never covers database I/O.
    pub(super) fn checkpoint(
        &self,
        id: &str,
        page: &mut Page,
        expected: &mut u64,
        stage: &str,
        status: &str,
        detail: &str,
    ) -> Result<()> {
        self.checkpoint_with_glossary(id, page, expected, stage, status, detail, None)
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn checkpoint_with_glossary(
        &self,
        id: &str,
        page: &mut Page,
        expected: &mut u64,
        stage: &str,
        status: &str,
        detail: &str,
        glossary: Option<crate::glossary::ExtractionCheckpoint>,
    ) -> Result<()> {
        let _op = self.operations.lock();
        let mut saved_job = {
            let state = self.state.lock();
            let e = state.jobs.get(id).context("Job no longer exists")?;
            anyhow::ensure!(
                e.job.status == "running" && !e.token.is_cancelled(),
                "Cancelled"
            );
            e.sampled()
        };
        if let Some(checkpoint) = glossary {
            saved_job.glossary_checkpoint = Some(checkpoint);
        }
        saved_job.page_revision = Some(*expected + 1);
        saved_job.record_step(stage, status, detail, None);
        if !store::save_checkpoint(Path::new(&saved_job.project), page, *expected, &saved_job)? {
            return Err(crate::job_state::ExecutionError::error(
                crate::job_state::FailureKind::Revision,
                "Newer edits preserved; start the job again to use current settings.",
            ));
        }
        *expected = page.revision;
        let mut state = self.state.lock();
        let e = state.jobs.get_mut(id).context("Job no longer exists")?;
        e.job.glossary_checkpoint = saved_job.glossary_checkpoint;
        e.job.page_revision = Some(page.revision);
        e.job.record_step(stage, status, detail, None);
        let job = state.touch(id);
        drop(state);
        (self.notify)(job);
        Ok(())
    }
    pub fn recover(&self, path: &str) -> Result<()> {
        let _op = self.operations.lock();
        if self.state.lock().recovered.contains(path) {
            return Ok(());
        }
        let mut rows = store::job_rows(&store::connection(Path::new(path))?)?;
        let mut jobs = rows.jobs;
        let order: HashMap<_, _> = store::page_order(Path::new(path))?.into_iter().collect();
        let mut changed = vec![];
        for job in &mut jobs {
            job.project = path.into();
            if ["running", "queued"].contains(&job.status.as_str()) {
                job.status = "paused".into();
                job.stage_detail = "Interrupted; resume to continue".into();
                changed.push(job.clone());
            }
        }
        let mut state = self.state.lock();
        let mut accepted = vec![];
        // Storage returns newest first. Insert oldest first to preserve ties in creation time.
        for job in jobs.into_iter().rev() {
            if let Some(existing) = state.jobs.get(&job.id) {
                if existing.job.project != path {
                    rows.errors.push(format!(
                        "Job {} conflicts with another book; its row remains unchanged",
                        job.id
                    ));
                }
                continue;
            }
            let key = (path.into(), job.page_id.clone());
            state.latest.insert(key, job.id.clone());
            let id = job.id.clone();
            let page_order = *order.get(&job.page_id).unwrap_or(&usize::MAX);
            let requirements_key = crate::job_view::JobView::requirements_key(&job);
            state.jobs.insert(
                id.clone(),
                Entry {
                    job,
                    requirements_key,
                    token: CancellationToken::new(),
                    sequence: 0,
                    page_order,
                    clock: None,
                },
            );
            state.touch(&id);
            accepted.push(id);
        }
        state.recovered.insert(path.into());
        drop(state);
        let accepted_ids: HashSet<_> = accepted.iter().collect();
        changed.retain(|j| accepted_ids.contains(&j.id));
        rows.errors
            .extend(self.persist_notify(changed).into_iter().map(|e| e.message));
        for id in accepted {
            if let Some(job) = self.job(&id) {
                (self.notify)(job);
            }
        }
        anyhow::ensure!(rows.errors.is_empty(), "{}", rows.errors.join("\n"));
        Ok(())
    }
    pub fn enqueue(
        &self,
        path: &str,
        ids: &[String],
        provider: &ProviderProfile,
    ) -> Result<Vec<Job>> {
        let settings = store::translation_settings(Path::new(path))?;
        self.enqueue_configured(path, ids, provider, &settings)
    }
    pub fn enqueue_configured(
        &self,
        path: &str,
        ids: &[String],
        provider: &ProviderProfile,
        settings: &TranslationSettings,
    ) -> Result<Vec<Job>> {
        self.enqueue_captured(
            path,
            ids,
            provider,
            settings,
            JobKind::Translation,
            &self.model_directory.lock().clone(),
        )
    }
    pub fn enqueue_cleanup(
        &self,
        path: &str,
        ids: &[String],
        settings: &TranslationSettings,
    ) -> Result<Vec<Job>> {
        self.enqueue_captured(
            path,
            ids,
            &ProviderProfile::default(),
            settings,
            JobKind::Cleanup,
            &self.model_directory.lock().clone(),
        )
    }
    pub fn enqueue_captured(
        &self,
        path: &str,
        ids: &[String],
        provider: &ProviderProfile,
        settings: &TranslationSettings,
        kind: JobKind,
        directory: &Path,
    ) -> Result<Vec<Job>> {
        self.enqueue_internal(path, ids, provider, settings, kind, directory, None)
    }
    // Mirrors enqueue_preparation, with the additional captured translation-service profile.
    #[allow(clippy::too_many_arguments)]
    pub fn enqueue_fresh(
        &self,
        path: &str,
        page_id: &str,
        expected: u64,
        replace: bool,
        provider: &ProviderProfile,
        settings: &TranslationSettings,
        directory: &Path,
    ) -> Result<Vec<Job>> {
        self.enqueue_internal(
            path,
            &[page_id.into()],
            provider,
            settings,
            JobKind::Translation,
            directory,
            Some((expected, replace)),
        )
    }
    pub fn enqueue_preparation(
        &self,
        path: &str,
        page_id: &str,
        expected: u64,
        replace: bool,
        settings: &TranslationSettings,
        directory: &Path,
    ) -> Result<Vec<Job>> {
        anyhow::ensure!(
            settings.mode == "local",
            "Choose Local OCR to prepare a page manually"
        );
        self.enqueue_internal(
            path,
            &[page_id.into()],
            &ProviderProfile::default(),
            settings,
            JobKind::Preparation,
            directory,
            Some((expected, replace)),
        )
    }
    #[allow(
        clippy::too_many_arguments,
        reason = "Explicit processing inputs preserve the shared production/benchmark and captured-job contracts"
    )]
    pub(super) fn enqueue_internal(
        &self,
        path: &str,
        ids: &[String],
        provider: &ProviderProfile,
        settings: &TranslationSettings,
        kind: JobKind,
        directory: &Path,
        preparation: Option<(u64, bool)>,
    ) -> Result<Vec<Job>> {
        let provider = crate::instructions::capture(provider, settings)?;
        self.recover(path)?;
        let _op = self.operations.lock();
        if let Some((expected, replace)) = preparation {
            let page = store::page(Path::new(path), &ids[0])?;
            anyhow::ensure!(
                page.revision == expected,
                "Page changed; review it and confirm preparation again"
            );
            anyhow::ensure!(
                replace
                    || (page.regions.is_empty()
                        && page.rendered.is_none()
                        && page.cleanup.is_none()
                        && page.background.is_none()),
                "Confirm replacement of the existing page regions and edits"
            );
        }
        let wanted: HashSet<_> = ids.iter().collect();
        let order = store::page_order(Path::new(path))?;
        // Reader/automatic submissions honor omission. Explicit fresh page requests,
        // preparation, cleanup, and already-captured jobs keep their own intent.
        let omitted: HashSet<String> = if kind == JobKind::Translation && preparation.is_none() {
            crate::library::open(Path::new(path))?
                .omitted_page_ids
                .into_iter()
                .collect()
        } else {
            HashSet::new()
        };
        let state = self.state.lock();
        if state.blocked.contains(path) {
            bail!("Book organization is being applied");
        }
        let mut jobs = vec![];
        let mut positions = HashMap::new();
        let submission = CapturedSubmission {
            path,
            provider: &provider,
            settings,
            kind,
            directory,
        };
        for (id, position) in order {
            if omitted.contains(&id) {
                continue;
            }
            if wanted.contains(&id) && state.editing.contains(&(path.into(), id.clone())) {
                bail!("This page is saving confirmed edits; try again when saving finishes");
            }
            if !wanted.contains(&id) || state.ongoing.contains_key(&(path.into(), id.clone())) {
                continue;
            }
            positions.insert(id.clone(), position);
            jobs.push(self.captured_job(&submission, &id, preparation.map(|p| p.0)));
        }
        drop(state);
        self.publish_enqueued(path, jobs, &positions)
    }
    pub(super) fn captured_job(
        &self,
        submission: &CapturedSubmission<'_>,
        page_id: &str,
        revision: Option<u64>,
    ) -> Job {
        Job {
            origin: if submission.kind != JobKind::Translation || revision.is_some() {
                crate::job_state::JobOrigin::Explicit
            } else {
                crate::job_state::JobOrigin::Reader
            },
            settings_captured: false,
            attempt: 0,
            failure_kind: None,
            id: uid(),
            project: submission.path.into(),
            page_id: page_id.into(),
            status: if self.is_held() { "paused" } else { "queued" }.into(),
            stage: "waiting".into(),
            stage_detail: if self.is_held() { "Jobs stopped" } else { "" }.into(),
            error: None,
            created: now(),
            elapsed_ms: 0,
            device: String::new(),
            settings: submission.settings.clone(),
            provider: submission.provider.clone(),
            kind: submission.kind,
            cleanup_model: None,
            model_directory: submission.directory.to_string_lossy().into(),
            steps: vec![],
            page_revision: revision,
            fresh: revision.is_some(),
            glossary_checkpoint: None,
        }
    }
    // Caller holds the operation gate. Persist every new job before making any runnable.
    pub(super) fn publish_enqueued(
        &self,
        path: &str,
        jobs: Vec<Job>,
        positions: &HashMap<String, usize>,
    ) -> Result<Vec<Job>> {
        self.publish_enqueued_at_commit(path, jobs, positions, || {})
    }
    fn publish_enqueued_at_commit(
        &self,
        path: &str,
        mut jobs: Vec<Job>,
        positions: &HashMap<String, usize>,
        committed: impl FnOnce(),
    ) -> Result<Vec<Job>> {
        if jobs.is_empty() {
            return Ok(jobs);
        }
        store::save_jobs(Path::new(path), &jobs)?;
        committed();
        // Stop can arrive during the transaction. The dispatch gate is already shut.
        let pause_write = self.is_held() && jobs.iter().any(|j| j.status == "queued");
        if pause_write {
            for job in &mut jobs {
                job.status = "paused".into();
                job.stage_detail = "Jobs stopped".into();
            }
        }
        let mut state = self.state.lock();
        for job in &jobs {
            state
                .latest
                .insert((path.into(), job.page_id.clone()), job.id.clone());
            state.jobs.insert(
                job.id.clone(),
                Entry {
                    job: job.clone(),
                    requirements_key: crate::job_view::JobView::requirements_key(job),
                    token: CancellationToken::new(),
                    sequence: 0,
                    page_order: positions[&job.page_id],
                    clock: None,
                },
            );
            state.touch(&job.id);
            if job.status == "queued" {
                state.pending.push_back(job.id.clone());
            }
        }
        state.order_pending();
        drop(state);
        if pause_write {
            // The insertion already committed. Retain every identity even if the later
            // pause write fails; the global dispatch hold remains in effect.
            self.persist_notify(jobs.clone());
            for job in &mut jobs {
                if let Some(current) = self.job(&job.id) {
                    *job = current;
                }
            }
        } else {
            for job in &jobs {
                (self.notify)(job.clone());
            }
        }
        Ok(jobs)
    }
    /// Snapshot every owner, including region-only requests and confirmed edits.
    pub fn operation_books(&self) -> HashSet<String> {
        let state = self.state.lock();
        state
            .jobs
            .values()
            .map(|e| e.job.project.clone())
            .chain(state.editing.iter().map(|(book, _)| book.clone()))
            .chain(
                state
                    .regions
                    .values()
                    .map(|operation| operation.book.clone()),
            )
            .collect()
    }
    /// A single identity lookup avoids cloning all captured settings for one control.
    pub fn job(&self, id: &str) -> Option<Job> {
        self.state.lock().jobs.get(id).map(Entry::sampled)
    }
    pub fn request_job_control(&self, id: &str) -> Result<(u64, u64)> {
        let mut state = self.state.lock();
        anyhow::ensure!(state.jobs.contains_key(id), "Job not found");
        let generation = state.control_intents.entry(id.into()).or_default();
        *generation += 1;
        Ok((*generation, self.control_serial.load(Ordering::SeqCst)))
    }
    pub fn control(&self, id: &str, action: &str) -> Result<()> {
        let intent = self.request_job_control(id)?;
        self.control_requested(id, action, intent)
    }
    pub fn control_requested(&self, id: &str, action: &str, intent: (u64, u64)) -> Result<()> {
        use crate::job_state::JobAction;
        let action = JobAction::parse(action)?;
        let _op = self.operations.lock();
        let prepared = if matches!(action, JobAction::Resume | JobAction::Retry) {
            Some(self.rebase_for_resume(&self.job(id).context("Job not found")?)?)
        } else {
            None
        };
        let mut state = self.state.lock();
        anyhow::ensure!(
            state.control_intents.get(id) == Some(&intent.0)
                && self.control_serial.load(Ordering::SeqCst) == intent.1,
            "Job control was superseded"
        );
        let e = state.jobs.get(id).context("Job not found")?;
        anyhow::ensure!(
            !["complete", "cancelled"].contains(&e.job.status.as_str()),
            "Completed or cancelled jobs cannot be changed"
        );
        if state.blocked.contains(&e.job.project) {
            bail!("Book organization is being applied");
        }
        if matches!(action, JobAction::Resume | JobAction::Retry) {
            if self.is_held() {
                bail!("Use Start all to resume jobs");
            }
            if state
                .ongoing
                .get(&(e.job.project.clone(), e.job.page_id.clone()))
                .is_some_and(|v| v != id)
            {
                bail!("This page already has an ongoing translation job");
            }
            if state.active.contains(id) {
                bail!("Job is still running or stopping");
            }
        }
        let e = state.jobs.get_mut(id).unwrap();
        match action {
            JobAction::Pause | JobAction::Cancel => {
                e.token.cancel();
                e.job.status = if action == JobAction::Pause {
                    "paused"
                } else {
                    "cancelled"
                }
                .into();
                state.restart.remove(id);
                state.pending.retain(|v| v != id);
            }
            JobAction::Resume | JobAction::Retry => {
                if let Some(job) = prepared {
                    e.job = job;
                    e.requirements_key = crate::job_view::JobView::requirements_key(&e.job);
                }
                e.token = CancellationToken::new();
                e.job.status = "queued".into();
                e.job.stage = "waiting".into();
                e.job.stage_detail.clear();
                e.job.error = None;
                if !state.pending.iter().any(|v| v == id) {
                    state.pending.push_back(id.into());
                }
            }
        }
        let job = state.touch(id);
        state.order_pending();
        drop(state);
        let errors = self.persist_notify(vec![job]);
        if let Some(error) = errors.first() {
            bail!("{}", error.message);
        }
        Ok(())
    }
    pub fn control_all(&self, action: &str) -> Result<ControlResult> {
        let request = self.request_control(action)?;
        self.control_all_requested(action, request)
    }
    pub fn request_control(&self, action: &str) -> Result<u64> {
        if !["start", "stop"].contains(&action) {
            bail!("Unknown jobs action");
        }
        let request = self.control_serial.fetch_add(1, Ordering::SeqCst) + 1;
        if action == "start" {
            let mut state = self.state.lock();
            state.bulk_intents = state.control_intents.clone();
        }
        if action == "stop" {
            self.hold();
        }
        Ok(request)
    }
    pub fn control_all_requested(&self, action: &str, request: u64) -> Result<ControlResult> {
        self.control_all_ready(action, request, &HashMap::new())
    }
    pub fn control_all_ready(
        &self,
        action: &str,
        request: u64,
        blocked: &HashMap<String, String>,
    ) -> Result<ControlResult> {
        let _op = self.operations.lock();
        let candidates: Vec<_> = if action == "start" {
            self.state
                .lock()
                .jobs
                .values()
                .filter(|e| ["paused", "queued", "failed"].contains(&e.job.status.as_str()))
                .map(|e| e.job.clone())
                .collect()
        } else {
            vec![]
        };
        let mut prepared = self.resume_plans(candidates);
        let mut state = self.state.lock();
        if request != self.control_serial.load(Ordering::SeqCst) {
            return Ok(ControlResult::default());
        }
        if action == "start" {
            self.held.store(false, Ordering::SeqCst);
        } else {
            state.restart.clear();
        }
        let mut ids: Vec<_> = state.jobs.keys().cloned().collect();
        ids.sort_by(|a, b| {
            state.jobs[a]
                .job
                .created
                .cmp(&state.jobs[b].job.created)
                .then(state.jobs[a].sequence.cmp(&state.jobs[b].sequence))
        });
        let mut changed = vec![];
        let mut result = ControlResult::default();
        let mut scheduled_ids = HashSet::new();
        let mut restarting_ids = HashSet::new();
        let mut omitted_ids = HashSet::new();
        let mut pending: HashSet<_> = state.pending.iter().cloned().collect();
        for id in ids {
            if action == "start"
                && state.control_intents.get(&id).copied().unwrap_or(0)
                    != state.bulk_intents.get(&id).copied().unwrap_or(0)
            {
                result.skipped += 1;
                continue;
            }
            let e = &state.jobs[&id];
            let key = (e.job.project.clone(), e.job.page_id.clone());
            if state.blocked.contains(&e.job.project) {
                result.skipped += 1;
                continue;
            }
            let eligible = if action == "stop" {
                ["running", "queued"].contains(&e.job.status.as_str())
            } else {
                ["paused", "queued", "failed"].contains(&e.job.status.as_str())
                    && state.ongoing.get(&key).is_none_or(|v| v == &id)
                    && (e.job.status != "failed" || state.latest.get(&key) == Some(&id))
            };
            if !eligible {
                result.skipped += 1;
                continue;
            }
            let active = state.active.contains(&id);
            let mut restarting = false;
            let e = state.jobs.get_mut(&id).unwrap();
            if action == "start" && !active {
                match prepared.remove(&id) {
                    Some(Ok(None)) => {
                        e.token.cancel();
                        e.job.status = "cancelled".into();
                        e.job.error = None;
                        e.job.failure_kind = None;
                        e.job.stage_detail =
                            "Skipped: page excluded from automatic translation".into();
                        pending.remove(&id);
                        state.restart.remove(&id);
                        changed.push(state.touch(&id));
                        result.omitted += 1;
                        omitted_ids.insert(id.clone());
                        continue;
                    }
                    Some(Err(error)) => {
                        e.job.status = "paused".into();
                        e.job.error = Some(error.to_string());
                        e.job.failure_kind = Some(
                            error
                                .downcast_ref::<crate::job_state::ExecutionError>()
                                .map_or(crate::job_state::FailureKind::Storage, |e| e.kind),
                        );
                        pending.remove(&id);
                        result.blocked += 1;
                        result.errors.push(JobFailure {
                            id: id.clone(),
                            message: error.to_string(),
                        });
                        changed.push(state.touch(&id));
                        continue;
                    }
                    Some(Ok(Some(job))) => {
                        restarting = (e.job.settings_captured && !job.settings_captured)
                            || e.job.page_revision != job.page_revision;
                        e.job = job;
                        e.requirements_key = crate::job_view::JobView::requirements_key(&e.job);
                    }
                    None => {}
                }
            }
            if action == "start"
                && let Some(reason) = blocked.get(&id)
            {
                e.token.cancel();
                e.job.status = "paused".into();
                e.job.error = Some(reason.clone());
                e.job.failure_kind = Some(crate::job_state::FailureKind::Requirements);
                e.job.stage_detail = "Requirements unavailable".into();
                pending.remove(&id);
                state.restart.remove(&id);
                result.errors.push(JobFailure {
                    id: id.clone(),
                    message: reason.clone(),
                });
                result.blocked += 1;
                changed.push(state.touch(&id));
                continue;
            }
            if restarting {
                result.restarting += 1;
                restarting_ids.insert(id.clone());
            }
            if action == "stop" {
                e.token.cancel();
                e.job.status = "paused".into();
                e.job.stage_detail = "Jobs stopped".into();
                pending.remove(&id);
            } else if active {
                state.restart.insert(id.clone());
                if !restarting {
                    result.scheduled += 1;
                    scheduled_ids.insert(id.clone());
                }
            } else {
                e.token = CancellationToken::new();
                e.job.status = "queued".into();
                e.job.stage = "waiting".into();
                e.job.stage_detail.clear();
                e.job.error = None;
                if !restarting {
                    result.scheduled += 1;
                    scheduled_ids.insert(id.clone());
                }
                if pending.insert(id.clone()) {
                    state.pending.push_back(id.clone());
                }
            }
            changed.push(state.touch(&id));
        }
        state.pending.retain(|id| pending.contains(id));
        state.order_pending();
        drop(state);
        result.changed = changed.len();
        let failures = self.persist_notify(changed);
        result.changed = result.changed.saturating_sub(failures.len());
        for failure in &failures {
            result.scheduled -= usize::from(scheduled_ids.remove(&failure.id));
            result.restarting -= usize::from(restarting_ids.remove(&failure.id));
            result.omitted -= usize::from(omitted_ids.remove(&failure.id));
            if !result.errors.iter().any(|e| e.id == failure.id) {
                result.blocked += 1;
            }
        }
        result.errors.extend(failures);
        Ok(result)
    }
    pub async fn pause_for_edit(
        self: &Arc<Self>,
        path: &str,
        page_id: &str,
    ) -> Result<PageEditGuard> {
        let key = (path.to_owned(), page_id.to_owned());
        let mut state = self.state.lock();
        anyhow::ensure!(
            !state.blocked.contains(path),
            "Book organization is being applied"
        );
        anyhow::ensure!(
            !state.busy_book(path),
            "This book is processing another page; wait for it to finish"
        );
        anyhow::ensure!(
            state.active_books() < self.concurrent_books.load(Ordering::SeqCst),
            "Processing limit reached; wait for an active book to finish"
        );
        state.editing.insert(key.clone());
        Ok(PageEditGuard {
            engine: self.clone(),
            key,
        })
    }
    pub async fn quiesce(self: &Arc<Self>, path: &str) -> Result<()> {
        {
            let mut state = self.state.lock();
            anyhow::ensure!(
                !state.editing.iter().any(|(book, _)| book == path),
                "An Editor save is in progress; try again when it finishes"
            );
            if !state.blocked.insert(path.into()) {
                bail!("Book organization is already being applied");
            }
            for e in state.jobs.values().filter(|e| e.job.project == path) {
                e.token.cancel();
            }
        }
        // All errors, including a panicking blocking task, unwind the reservation.
        let mut reservation = BookReservation {
            engine: self.clone(),
            path: path.to_owned(),
        };
        let worker = self.clone();
        let owned = path.to_owned();
        let errors = tokio::task::spawn_blocking(move || {
            let _op = worker.operations.lock();
            let mut state = worker.state.lock();
            let ids: Vec<_> = state
                .jobs
                .values()
                .filter(|e| {
                    e.job.project == owned && ["queued", "running"].contains(&e.job.status.as_str())
                })
                .map(|e| e.job.id.clone())
                .collect();
            let mut jobs = vec![];
            for id in ids {
                let e = state.jobs.get_mut(&id).unwrap();
                e.token.cancel();
                e.job.status = "paused".into();
                e.job.stage_detail = "Paused for book changes".into();
                state.restart.remove(&id);
                jobs.push(state.touch(&id));
            }
            drop(state);
            worker.persist_notify(jobs)
        })
        .await?;
        if let Some(error) = errors.first() {
            self.state.lock().blocked.remove(path);
            bail!("{}", error.message);
        }
        loop {
            let active = {
                let state = self.state.lock();
                state
                    .active
                    .iter()
                    .any(|id| state.jobs.get(id).is_some_and(|e| e.job.project == path))
            };
            if !active {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        reservation.path.clear();
        Ok(())
    }
    pub async fn reserve_book(self: &Arc<Self>, path: &str) -> Result<BookReservation> {
        self.quiesce(path).await?;
        Ok(BookReservation {
            engine: self.clone(),
            path: path.to_owned(),
        })
    }
    pub fn release_book(&self, path: &str, keep: Option<&[String]>) {
        let _op = self.operations.lock();
        let mut state = self.state.lock();
        let order: HashMap<_, _> = keep
            .unwrap_or_default()
            .iter()
            .enumerate()
            .map(|(i, id)| (id.clone(), i))
            .collect();
        let removed: Vec<_> = state
            .jobs
            .values()
            .filter(|e| e.job.project == path && !order.contains_key(&e.job.page_id))
            .map(|e| e.job.clone())
            .collect();
        for job in &removed {
            state.jobs.remove(&job.id);
            state.restart.remove(&job.id);
            state.pending.retain(|id| id != &job.id);
            state.revision += 1;
        }
        for e in state.jobs.values_mut().filter(|e| e.job.project == path) {
            if let Some(position) = order.get(&e.job.page_id) {
                e.page_order = *position;
            }
        }
        state.blocked.remove(path);
        if keep.is_none() {
            state.recovered.remove(path);
        }
        state.reindex();
        state.order_pending();
        drop(state);
        for job in removed {
            (self.notify)(job);
        }
    }
    pub fn prioritize(&self, _path: &str, _ids: &[String]) {
        // Reader navigation cannot reorder queued pages within a book.
        self.state.lock().order_pending();
    }
}

#[cfg(test)]
mod book_concurrency_contracts {
    use super::*;
    #[test]
    fn round_robin_admits_every_waiting_book_before_a_second_page() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let tmp = tempfile::tempdir_in(root.join("test-output")).unwrap();
        let original = tmp.path().join("source.png");
        image::RgbImage::new(32, 32).save(&original).unwrap();
        let base = documents::import(&[original.to_string_lossy().into()])
            .unwrap()
            .remove(0);
        let pages = (0..2)
            .map(|n| {
                let mut p = base.clone();
                p.id = uid();
                p.number = n;
                p
            })
            .collect::<Vec<_>>();
        let e = Engine::new(
            root.join("assets/models"),
            &root.join("assets/fonts"),
            Arc::new(|_| {}),
            Arc::new(|_| bail!("No provider access")),
        )
        .unwrap();
        e.configure(&AppSettings {
            concurrent_books: 1,
            ..Default::default()
        });
        let mut books = Vec::new();
        for name in ["a", "b", "c"] {
            let book =
                store::create(&tmp.path().join(format!("{name}.umanga")), name, &pages).unwrap();
            e.enqueue(
                &book.path,
                &pages.iter().map(|p| p.id.clone()).collect::<Vec<_>>(),
                &ProviderProfile::default(),
            )
            .unwrap();
            books.push(book.path);
        }
        for n in 0..6 {
            let (job, token) = e.pick().unwrap();
            assert_eq!(job.project, books[n % 3]);
            assert_eq!(job.page_id, pages[n / 3].id);
            e.finish(&job.id, &token, Ok(()), 1);
        }
        assert!(e.pick().is_none());
    }
    #[tokio::test]
    async fn ownership_covers_all_page_operations_and_stopping_workers() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let tmp = tempfile::tempdir_in(root.join("test-output")).unwrap();
        let original = tmp.path().join("source.png");
        image::RgbImage::new(32, 32).save(&original).unwrap();
        let base = documents::import(&[original.to_string_lossy().into()])
            .unwrap()
            .remove(0);
        let pages = (0..3)
            .map(|n| {
                let mut p = base.clone();
                p.id = uid();
                p.number = n;
                p
            })
            .collect::<Vec<_>>();
        let a = store::create(&tmp.path().join("a.umanga"), "A", &pages).unwrap();
        let b = store::create(&tmp.path().join("b.umanga"), "B", &pages[..1]).unwrap();
        let e = Engine::new(
            root.join("assets/models"),
            &root.join("assets/fonts"),
            Arc::new(|_| {}),
            Arc::new(|_| bail!("No provider access")),
        )
        .unwrap();
        let profile = ProviderProfile::default();
        e.enqueue(
            &a.path,
            &[pages[1].id.clone(), pages[0].id.clone()],
            &profile,
        )
        .unwrap();
        e.enqueue(&b.path, &[pages[0].id.clone()], &profile)
            .unwrap();
        e.prioritize(&a.path, &[pages[1].id.clone()]);
        let (first, token) = e.pick().unwrap();
        assert_eq!(first.page_id, pages[0].id);
        let (other, other_token) = e.pick().unwrap();
        assert_eq!(other.project, b.path);
        assert!(e.pick().is_none());
        assert!(e.reserve_region(&uid(), &a.path, &pages[2].id).is_err());
        assert!(e.pause_for_edit(&a.path, &pages[2].id).await.is_err());
        e.control(&first.id, "cancel").unwrap();
        assert!(e.pick().is_none());
        let config = AppSettings {
            concurrent_books: 1,
            ..Default::default()
        };
        e.configure(&config);
        e.finish(&first.id, &token, Ok(()), 1);
        assert!(e.pick().is_none());
        e.finish(&other.id, &other_token, Ok(()), 1);
        let guard = e.pause_for_edit(&a.path, &pages[2].id).await.unwrap();
        assert!(e.pick().is_none());
        drop(guard);
        let region = e.reserve_region(&uid(), &a.path, &pages[2].id).unwrap();
        assert!(e.pick().is_none());
        drop(region);
        let (next, token) = e.pick().unwrap();
        assert_eq!(next.page_id, pages[1].id);
        e.finish(&next.id, &token, Ok(()), 1);
    }
}
