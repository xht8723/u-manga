//! First-execution configuration and explicit conflict recovery. No processing in readiness.
use super::*;
use crate::job_state::{ExecutionError, FailureKind, JobOrigin};
use std::sync::atomic::Ordering;

fn signature(job: &Job) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&(
        &job.settings,
        &job.provider,
        &job.model_directory,
    ))?)
}

impl Engine {
    fn configuration_from_book(
        &self,
        mut job: Job,
        book: &crate::library::Book,
        defaults: &AppSettings,
    ) -> Result<Job> {
        if self.application_configured.load(Ordering::SeqCst) {
            job.settings = crate::library::effective(book, defaults);
            job.provider = if job.kind == JobKind::Translation {
                defaults
                    .providers
                    .iter()
                    .find(|p| p.id == job.settings.provider_id)
                    .cloned()
                    .ok_or_else(|| {
                        ExecutionError::error(
                            FailureKind::Configuration,
                            "Select a translation service",
                        )
                    })?
            } else {
                ProviderProfile::default()
            };
            job.model_directory = defaults.model_directory().to_string_lossy().into();
        } else {
            let effective = crate::library::effective(book, defaults);
            if effective.source_language == job.settings.source_language
                && effective.target_language == job.settings.target_language
            {
                job.settings.glossary = if job.settings.glossary_enabled {
                    book.glossary.entries.clone()
                } else {
                    vec![]
                };
                job.settings.deepl_glossary_id = book.glossary.deepl_glossary_id.clone();
            }
        }
        job.provider = crate::instructions::capture(&job.provider, &job.settings)?;
        Ok(job)
    }
    fn current_configuration(&self, job: Job) -> Result<Job> {
        let book = crate::library::open(Path::new(&job.project))?;
        self.configuration_from_book(job, &book, &self.application_settings.lock())
    }

    /// One metadata/revision snapshot per book; bulk controls must not open SQLite per page.
    pub fn resume_plans(&self, jobs: Vec<Job>) -> HashMap<String, Result<Option<Job>>> {
        let mut groups: HashMap<String, Vec<Job>> = HashMap::new();
        for job in jobs {
            groups.entry(job.project.clone()).or_default().push(job);
        }
        let defaults = self.application_settings.lock().clone();
        let mut output = HashMap::new();
        for (path, jobs) in groups {
            let snapshot = (|| -> Result<_> {
                let mut c = store::connection(Path::new(&path))?;
                let tx = c.transaction()?;
                let book = crate::library::get(&tx)?;
                let revisions = {
                    let mut q = tx.prepare("SELECT id,revision FROM pages")?;
                    q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))?
                        .collect::<rusqlite::Result<HashMap<_, _>>>()?
                };
                tx.commit()?;
                Ok((book, revisions))
            })();
            for job in jobs {
                let id = job.id.clone();
                let next = (|| -> Result<Option<Job>> {
                    let (book, revisions) = snapshot
                        .as_ref()
                        .map_err(|e| ExecutionError::error(FailureKind::Storage, e.to_string()))?;
                    let revision = *revisions
                        .get(&job.page_id)
                        .context("Page no longer belongs to this book")?;
                    let conflict = job.page_revision.is_some_and(|old| old != revision)
                        || matches!(
                            job.failure_kind,
                            Some(FailureKind::Revision | FailureKind::Configuration)
                        );
                    let mut next = job;
                    if conflict {
                        next.attempt = next
                            .attempt
                            .checked_add(1)
                            .context("Job attempt limit reached")?;
                        next.settings_captured = false;
                        next.steps.clear();
                        next.glossary_checkpoint = None;
                        next.cleanup_model = None;
                        next.failure_kind = None;
                        next.page_revision = Some(revision);
                    }
                    if !next.settings_captured {
                        if next.origin != JobOrigin::Explicit
                            && book.omitted_page_ids.contains(&next.page_id)
                        {
                            return Ok(None);
                        }
                        next = self.configuration_from_book(next, book, &defaults)?;
                    }
                    Ok(Some(next))
                })();
                output.insert(id, next);
            }
        }
        output
    }

    pub fn job_omitted(&self, job: &Job) -> Result<bool> {
        Ok(!job.settings_captured
            && job.origin != JobOrigin::Explicit
            && crate::library::open(Path::new(&job.project))?
                .omitted_page_ids
                .contains(&job.page_id))
    }

    /// A prospective restart does not modify the job or page, and does not freeze waiting settings.
    pub fn prospective_job(&self, job: Job) -> Result<Job> {
        self.resume_plans(vec![job.clone()])
            .remove(&job.id)
            .context("Job no longer exists")?
            .map(|next| next.unwrap_or(job))
    }

    /// Caller owns the operation gate. Restart remains pending until the dispatch snapshot commits.
    pub(super) fn rebase_for_resume(&self, job: &Job) -> Result<Job> {
        let mut next = self.prospective_job(job.clone())?;
        next.error = None;
        Ok(next)
    }

    pub(super) async fn begin_execution(
        self: &Arc<Self>,
        job: &Job,
        cancel: &CancellationToken,
    ) -> Result<Option<Job>> {
        loop {
            anyhow::ensure!(!cancel.is_cancelled(), "Cancelled");
            let engine = self.clone();
            let original = job.clone();
            let candidate = tokio::task::spawn_blocking(move || -> Result<Option<Job>> {
                if engine.job_omitted(&original)? {
                    return Ok(None);
                }
                let revision =
                    store::page(Path::new(&original.project), &original.page_id)?.revision;
                if original.page_revision.is_some_and(|old| old != revision) {
                    return Err(ExecutionError::error(
                        FailureKind::Revision,
                        "Page changed; start the job again to use current settings.",
                    ));
                }
                let mut next = if original.settings_captured {
                    original
                } else {
                    engine.current_configuration(original)?
                };
                next.page_revision = Some(revision);
                Ok(Some(next))
            })
            .await??;
            let Some(mut candidate) = candidate else {
                self.skip_omitted(&job.id)?;
                return Ok(None);
            };
            let before = signature(&candidate)?;
            if !candidate.settings_captured
                && self.application_configured.load(Ordering::SeqCst)
                && candidate.kind == JobKind::Translation
            {
                candidate.provider.thinking_policy = Some(tokio::select! {
                    _ = cancel.cancelled() => bail!("Cancelled"),
                    policy = crate::thinking::resolve(&candidate.provider) => policy,
                });
            }
            let readiness = tokio::select! {
                _ = cancel.cancelled() => bail!("Cancelled"),
                result = self.check_requirements(candidate.clone()) => result,
            };
            anyhow::ensure!(!cancel.is_cancelled(), "Cancelled");
            // Settings may have changed while model/credential checks were in flight.
            if !job.settings_captured {
                let engine = self.clone();
                let original = job.clone();
                let latest =
                    tokio::task::spawn_blocking(move || engine.current_configuration(original))
                        .await??;
                if signature(&latest)? != before {
                    continue;
                }
            }
            if let Err(error) = readiness {
                return Err(ExecutionError::error(
                    FailureKind::Requirements,
                    format!("{error:#}"),
                ));
            }
            let engine = self.clone();
            let token = cancel.clone();
            let old = job.clone();
            let committed = tokio::task::spawn_blocking(move || -> Result<Option<Job>> {
                let _operation = engine.operations.lock();
                {
                    let state = engine.state.lock();
                    let entry = state.jobs.get(&old.id).context("Job no longer exists")?;
                    anyhow::ensure!(
                        state.active.contains(&old.id)
                            && entry.job.status == "running"
                            && !token.is_cancelled(),
                        "Cancelled"
                    );
                }
                let mut c = store::connection(Path::new(&old.project))?;
                let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                let defaults = engine.application_settings.lock();
                let book = crate::library::get(&tx)?;
                if !old.settings_captured {
                    if old.origin != JobOrigin::Explicit
                        && book.omitted_page_ids.contains(&old.page_id)
                    {
                        drop(defaults);
                        drop(tx);
                        drop(_operation);
                        engine.skip_omitted(&old.id)?;
                        return Ok(Some(engine.job(&old.id).context("Job no longer exists")?));
                    }
                    if engine.application_configured.load(Ordering::SeqCst) {
                        let mut latest = old.clone();
                        latest.settings = crate::library::effective(&book, &defaults);
                        latest.provider = if old.kind == JobKind::Translation {
                            defaults
                                .providers
                                .iter()
                                .find(|p| p.id == latest.settings.provider_id)
                                .cloned()
                                .ok_or_else(|| {
                                    ExecutionError::error(
                                        FailureKind::Configuration,
                                        "Select a translation service",
                                    )
                                })?
                        } else {
                            ProviderProfile::default()
                        };
                        latest.provider =
                            crate::instructions::capture(&latest.provider, &latest.settings)?;
                        latest.model_directory =
                            defaults.model_directory().to_string_lossy().into();
                        if signature(&latest)? != before {
                            return Ok(None);
                        }
                    }
                }
                let revision: u64 = tx.query_row(
                    "SELECT revision FROM pages WHERE id=?1",
                    [&old.page_id],
                    |r| r.get(0),
                )?;
                if candidate.page_revision != Some(revision) {
                    return Err(ExecutionError::error(
                        FailureKind::Revision,
                        "Page changed; start the job again to use current settings.",
                    ));
                }
                candidate.settings_captured = true;
                candidate.failure_kind = None;
                anyhow::ensure!(!token.is_cancelled(), "Cancelled");
                tx.execute(
                    "INSERT OR REPLACE INTO jobs VALUES(?1,?2)",
                    rusqlite::params![candidate.id, crate::job_validation::serialize(&candidate)?],
                )?;
                tx.commit()?;
                drop(defaults);
                let mut state = engine.state.lock();
                let entry = state
                    .jobs
                    .get_mut(&old.id)
                    .context("Job no longer exists")?;
                entry.requirements_key = crate::job_view::JobView::requirements_key(&candidate);
                entry.job = candidate;
                let saved = state.touch(&old.id);
                drop(state);
                (engine.notify)(saved.clone());
                Ok(Some(saved))
            })
            .await??;
            if let Some(saved) = committed {
                return Ok((saved.status != "cancelled").then_some(saved));
            }
        }
    }

    fn skip_omitted(&self, id: &str) -> Result<()> {
        let _operation = self.operations.lock();
        let mut state = self.state.lock();
        let e = state.jobs.get_mut(id).context("Job no longer exists")?;
        if e.job.status != "running" || e.token.is_cancelled() {
            bail!("Cancelled");
        }
        e.token.cancel();
        e.job.status = "cancelled".into();
        e.job.stage_detail = "Skipped: page excluded from automatic translation".into();
        e.job.error = None;
        e.job.failure_kind = None;
        let job = state.touch(id);
        drop(state);
        if let Some(error) = self.persist_notify(vec![job]).first() {
            bail!("{}", error.message);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn fixture(count: usize) -> (tempfile::TempDir, Project, Arc<Engine>, AppSettings) {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let dir = tempfile::tempdir_in(root.join("test-output")).unwrap();
        let source = dir.path().join("original.png");
        image::RgbImage::new(64, 96).save(&source).unwrap();
        let page = documents::import(&[source.to_string_lossy().into()])
            .unwrap()
            .remove(0);
        let pages = (0..count)
            .map(|number| Page {
                id: uid(),
                number,
                ..page.clone()
            })
            .collect::<Vec<_>>();
        let book = store::create(&dir.path().join("book.umanga"), "Job settings", &pages).unwrap();
        let engine = Engine::new(
            dir.path().join("models"),
            &root.join("assets/fonts"),
            Arc::new(|_| {}),
            Arc::new(|_| bail!("No provider requests")),
        )
        .unwrap();
        let provider = ProviderProfile {
            service: "deepl".into(),
            name: "Old service".into(),
            ..Default::default()
        };
        let mut settings = AppSettings {
            library_directory: dir.path().to_string_lossy().into(),
            providers: vec![provider.clone()],
            ..Default::default()
        };
        settings.translation.provider_id = provider.id;
        engine.configure(&settings);
        (dir, book, engine, settings)
    }

    fn enqueue(book: &Project, engine: &Engine, settings: &AppSettings) -> Job {
        engine
            .enqueue_configured(
                &book.path,
                &[book.pages[0].id.clone()],
                &settings.providers[0],
                &settings.translation,
            )
            .unwrap()
            .remove(0)
    }

    #[tokio::test]
    async fn waiting_uses_latest_and_started_resume_keeps_frozen_snapshot() {
        let (_dir, book, engine, mut settings) = fixture(1);
        let job = enqueue(&book, &engine, &settings);
        settings.translation.target_language = "fr".into();
        settings.providers[0].name = "New service".into();
        settings.providers[0].model = "new-model".into();
        settings.providers[0].instructions.text_translation = Some("Current guidance".into());
        settings.translation.ocr = "pp".into();
        settings.translation.cleanup.method = CleanupMethod::Solid;
        engine.configure(&settings);
        let (picked, token) = engine.pick().unwrap();
        let started = engine
            .begin_execution(&picked, &token)
            .await
            .unwrap()
            .unwrap();
        assert!(started.settings_captured);
        assert_eq!(started.settings.target_language, "fr");
        assert_eq!(started.provider.name, "New service");
        engine.control(&job.id, "pause").unwrap();
        engine.finish(&job.id, &token, Err(anyhow::anyhow!("Cancelled")), 1);
        settings.translation.target_language = "de".into();
        engine.configure(&settings);
        engine.control(&job.id, "resume").unwrap();
        let (picked, token) = engine.pick().unwrap();
        let resumed = engine
            .begin_execution(&picked, &token)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resumed.settings.target_language, "fr");
        assert_eq!(
            serde_json::to_value(&resumed.settings).unwrap(),
            serde_json::to_value(&started.settings).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&resumed.provider).unwrap(),
            serde_json::to_value(&started.provider).unwrap()
        );
        assert_eq!(
            store::jobs(Path::new(&book.path)).unwrap()[0]
                .settings
                .target_language,
            "fr"
        );
        engine.finish(&job.id, &token, Ok(()), 2);
    }

    #[tokio::test]
    async fn settings_changed_during_validation_are_rechecked_before_capture() {
        let (_dir, book, engine, mut settings) = fixture(1);
        enqueue(&book, &engine, &settings);
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let checks = Arc::new(AtomicUsize::new(0));
        let (a, b, c) = (entered.clone(), release.clone(), checks.clone());
        engine.set_requirements(Arc::new(move |_| {
            let (a, b, c) = (a.clone(), b.clone(), c.clone());
            Box::pin(async move {
                if c.fetch_add(1, Ordering::SeqCst) == 0 {
                    a.notify_one();
                    b.notified().await;
                }
                Ok(())
            })
        }));
        let (picked, token) = engine.pick().unwrap();
        let worker = engine.clone();
        let task = tokio::spawn(async move { worker.begin_execution(&picked, &token).await });
        entered.notified().await;
        settings.translation.context_pages = 7;
        settings.providers[0].instructions.text_translation = Some("Updated instructions".into());
        engine.configure(&settings);
        release.notify_one();
        let saved = task.await.unwrap().unwrap().unwrap();
        assert_eq!(saved.settings.context_pages, 7);
        assert_eq!(
            saved.provider.instructions.text_translation.as_deref(),
            Some("Updated instructions")
        );
        assert_eq!(checks.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn omission_does_not_interrupt_started_work_and_skips_only_unstarted_automatic_jobs() {
        let (dir, book, engine, settings) = fixture(2);
        engine
            .enqueue_configured(
                &book.path,
                &book.pages.iter().map(|p| p.id.clone()).collect::<Vec<_>>(),
                &settings.providers[0],
                &settings.translation,
            )
            .unwrap();
        let (picked, token) = engine.pick().unwrap();
        let started = engine
            .begin_execution(&picked, &token)
            .await
            .unwrap()
            .unwrap();
        let ids = book.pages.iter().map(|p| p.id.clone()).collect::<Vec<_>>();
        crate::library::update_omissions(dir.path(), Path::new(&book.path), &[], &ids).unwrap();
        assert!(!token.is_cancelled());
        assert!(!engine.job_omitted(&started).unwrap());
        assert_eq!(
            store::page(Path::new(&book.path), &started.page_id)
                .unwrap()
                .revision,
            0
        );
        engine.finish(&started.id, &token, Ok(()), 1);
        let (next, token) = engine.pick().unwrap();
        assert!(
            engine
                .begin_execution(&next, &token)
                .await
                .unwrap()
                .is_none()
        );
        engine.finish(&next.id, &token, Ok(()), 1);
        let skipped = engine.job(&next.id).unwrap();
        assert_eq!(skipped.status, "cancelled");
        assert!(skipped.error.is_none());
        let explicit = engine
            .enqueue_fresh(
                &book.path,
                &book.pages[1].id,
                0,
                true,
                &settings.providers[0],
                &settings.translation,
                dir.path(),
            )
            .unwrap()
            .remove(0);
        assert!(!engine.job_omitted(&explicit).unwrap());
    }

    #[test]
    fn bulk_rebases_172_conflicts_and_later_individual_cancel_wins() {
        let (dir, book, engine, settings) = fixture(172);
        for p in &book.pages {
            engine
                .enqueue_fresh(
                    &book.path,
                    &p.id,
                    0,
                    true,
                    &settings.providers[0],
                    &settings.translation,
                    dir.path(),
                )
                .unwrap();
            let mut changed = p.clone();
            assert!(store::save_page(Path::new(&book.path), &mut changed, 0).unwrap());
        }
        let request = engine.request_control("start").unwrap();
        let cancelled = engine.list()[0].id.clone();
        engine.control(&cancelled, "cancel").unwrap();
        let result = engine.control_all_requested("start", request).unwrap();
        assert!(result.errors.is_empty());
        assert_eq!(result.scheduled, 0);
        assert_eq!(result.restarting, 171);
        assert_eq!(engine.job(&cancelled).unwrap().status, "cancelled");
        for job in engine.list().into_iter().filter(|j| j.status == "queued") {
            assert_eq!(job.page_revision, Some(1));
            assert_eq!(job.attempt, 1);
            let mut original = job.clone();
            original.attempt = 0;
            assert_ne!(
                request_cache_key("sample".into(), &original),
                request_cache_key("sample".into(), &job)
            );
            assert!(job.steps.is_empty());
            assert!(!job.settings_captured);
        }
    }

    #[tokio::test]
    async fn failed_capture_does_not_publish_snapshot_or_change_page() {
        let (_dir, book, engine, settings) = fixture(1);
        enqueue(&book, &engine, &settings);
        store::connection(Path::new(&book.path)).unwrap().execute_batch("CREATE TRIGGER fail_capture BEFORE INSERT ON jobs WHEN json_extract(NEW.data,'$.settingsCaptured')=1 BEGIN SELECT RAISE(ABORT,'injected capture failure'); END;").unwrap();
        let (job, token) = engine.pick().unwrap();
        assert!(engine.begin_execution(&job, &token).await.is_err());
        assert!(!engine.job(&job.id).unwrap().settings_captured);
        assert!(!store::jobs(Path::new(&book.path)).unwrap()[0].settings_captured);
        assert_eq!(
            store::page(Path::new(&book.path), &job.page_id)
                .unwrap()
                .revision,
            0
        );
    }
    #[tokio::test]
    async fn cancellation_during_validation_never_captures_or_starts() {
        let (_dir, book, engine, settings) = fixture(1);
        let job = enqueue(&book, &engine, &settings);
        let entered = Arc::new(tokio::sync::Notify::new());
        let signal = entered.clone();
        engine.set_requirements(Arc::new(move |_| {
            let signal = signal.clone();
            Box::pin(async move {
                signal.notify_one();
                std::future::pending::<Result<()>>().await
            })
        }));
        let (picked, token) = engine.pick().unwrap();
        let worker = engine.clone();
        let task = tokio::spawn(async move { worker.begin_execution(&picked, &token).await });
        entered.notified().await;
        engine.control(&job.id, "cancel").unwrap();
        assert!(task.await.unwrap().is_err());
        assert!(!engine.job(&job.id).unwrap().settings_captured);
        assert_eq!(engine.job(&job.id).unwrap().status, "cancelled");
    }

    #[tokio::test]
    async fn ordinary_requirement_failure_keeps_started_snapshot_on_retry() {
        let (_dir, book, engine, mut settings) = fixture(1);
        let job = enqueue(&book, &engine, &settings);
        let (picked, token) = engine.pick().unwrap();
        let saved = engine
            .begin_execution(&picked, &token)
            .await
            .unwrap()
            .unwrap();
        engine.finish(
            &job.id,
            &token,
            Err(ExecutionError::error(
                FailureKind::Requirements,
                "Missing model",
            )),
            1,
        );
        settings.translation.target_language = "de".into();
        engine.configure(&settings);
        let prospective = engine
            .prospective_job(engine.job(&job.id).unwrap())
            .unwrap();
        assert!(prospective.settings_captured);
        assert_eq!(
            prospective.settings.target_language,
            saved.settings.target_language
        );
    }

    #[test]
    fn bulk_persistence_failure_is_blocked_and_stop_supersedes_pending_start() {
        let (_dir, book, engine, settings) = fixture(1);
        let job = enqueue(&book, &engine, &settings);
        let start = engine.request_control("start").unwrap();
        engine.request_control("stop").unwrap();
        assert_eq!(
            engine
                .control_all_requested("start", start)
                .unwrap()
                .scheduled,
            0
        );
        store::connection(Path::new(&book.path)).unwrap().execute_batch("CREATE TRIGGER fail_job BEFORE INSERT ON jobs BEGIN SELECT RAISE(ABORT,'injected save failure'); END;").unwrap();
        let start = engine.request_control("start").unwrap();
        let result = engine.control_all_requested("start", start).unwrap();
        assert_eq!(result.scheduled, 0);
        assert_eq!(result.changed, 0);
        assert_eq!(result.blocked, 1);
        assert_eq!(
            engine.job(&job.id).unwrap().failure_kind,
            Some(FailureKind::Storage)
        );
    }
    #[tokio::test]
    async fn recovered_started_jobs_keep_snapshot_and_conflicted_detection_preserves_saved_page() {
        let (dir, book, engine, mut settings) = fixture(1);
        let job = enqueue(&book, &engine, &settings);
        let (picked, token) = engine.pick().unwrap();
        let saved = engine
            .begin_execution(&picked, &token)
            .await
            .unwrap()
            .unwrap();
        engine.control(&job.id, "pause").unwrap();
        engine.finish(&job.id, &token, Err(anyhow::anyhow!("Cancelled")), 1);
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let recovered = Engine::new(
            dir.path().join("models"),
            &root.join("assets/fonts"),
            Arc::new(|_| {}),
            Arc::new(|_| bail!("No provider calls")),
        )
        .unwrap();
        settings.translation.target_language = "fr".into();
        recovered.configure(&settings);
        recovered.recover(&book.path).unwrap();
        let next = recovered
            .prospective_job(recovered.job(&job.id).unwrap())
            .unwrap();
        assert!(next.settings_captured);
        assert_eq!(
            next.settings.target_language,
            saved.settings.target_language
        );
        recovered.control(&job.id, "cancel").unwrap();
        let fresh = recovered
            .enqueue_fresh(
                &book.path,
                &book.pages[0].id,
                0,
                true,
                &settings.providers[0],
                &settings.translation,
                dir.path(),
            )
            .unwrap()
            .remove(0);
        let mut page = store::page(Path::new(&book.path), &book.pages[0].id).unwrap();
        page.regions.push(Region {
            source: "Manual original".into(),
            target: "Manual translation".into(),
            bbox: [1., 1., 30., 30.],
            ..Default::default()
        });
        store::save_page(Path::new(&book.path), &mut page, 0).unwrap();
        let before = serde_json::to_value(&page).unwrap();
        recovered.control(&fresh.id, "resume").unwrap();
        let picked = recovered.pick().unwrap();
        let token = picked.1.clone();
        assert_eq!(picked.0.attempt, 1);
        let result = recovered.clone().process(&fresh.id, picked).await;
        assert!(result.is_err()); // No detector is installed in this isolated fixture.
        recovered.finish(&fresh.id, &token, result, 1);
        assert_eq!(
            serde_json::to_value(store::page(Path::new(&book.path), &page.id).unwrap()).unwrap(),
            before
        );
        assert_eq!(
            recovered.job(&fresh.id).unwrap().settings.target_language,
            "fr"
        );
    }
}
