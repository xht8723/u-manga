//! Page-job glossary learning. Provider work is async; durable transitions run on blocking workers.
use super::*;
use crate::glossary::{Extraction, ExtractionCheckpoint};
use anyhow::ensure;
use rusqlite::{OptionalExtension, params};

enum Update {
    Capture,
    Refresh,
    Extracted {
        ids: Vec<String>,
        result: Extraction,
        cache_key: String,
    },
    Complete(Option<String>),
}
impl Engine {
    pub(super) async fn capture_glossary(
        self: &Arc<Self>,
        job: &mut Job,
        revision: u64,
    ) -> Result<()> {
        if job.detects_glossary() && job.glossary_checkpoint.is_none() {
            *job = self
                .glossary_update(&job.id, revision, Update::Capture)
                .await?;
        }
        Ok(())
    }
    async fn glossary_update(
        self: &Arc<Self>,
        id: &str,
        revision: u64,
        update: Update,
    ) -> Result<Job> {
        let engine = self.clone();
        let id = id.to_owned();
        tokio::task::spawn_blocking(move || engine.glossary_update_sync(&id, revision, update))
            .await?
    }
    fn glossary_update_sync(&self, id: &str, revision: u64, update: Update) -> Result<Job> {
        let _operation = self.operations.lock();
        let (mut job, token) = {
            let state = self.state.lock();
            let entry = state.jobs.get(id).context("Job no longer exists")?;
            ensure!(
                state.active.contains(id)
                    && entry.job.status == "running"
                    && !entry.token.is_cancelled(),
                "Cancelled"
            );
            (entry.sampled(), entry.token.clone())
        };
        let path = PathBuf::from(&job.project);
        ensure!(path.is_file(), "Book no longer exists");
        let mut connection = store::connection(&path)?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current: u64 = tx.query_row(
            "SELECT revision FROM pages WHERE id=?1",
            [&job.page_id],
            |r| r.get(0),
        )?;
        ensure!(
            current == revision,
            "Page changed; glossary results were not applied"
        );
        let mut book = crate::library::get(&tx)?;
        let effective = crate::library::effective(&book, &self.application_settings.lock());
        let pair_matches = effective.source_language == job.settings.source_language
            && effective.target_language == job.settings.target_language;
        if matches!(update, Update::Capture | Update::Refresh) {
            if pair_matches {
                job.settings.glossary = if job.settings.glossary_enabled {
                    book.glossary.entries.clone()
                } else {
                    Vec::new()
                };
                job.settings.deepl_glossary_id = book.glossary.deepl_glossary_id.clone();
            }
            if matches!(update, Update::Capture) && job.glossary_checkpoint.is_none() {
                job.glossary_checkpoint = Some(ExtractionCheckpoint::default());
            }
        } else {
            let checkpoint = job
                .glossary_checkpoint
                .as_mut()
                .context("Missing glossary checkpoint")?;
            match update {
                Update::Extracted {
                    ids,
                    result,
                    cache_key,
                } => {
                    let data: String =
                        tx.query_row("SELECT data FROM pages WHERE id=?1", [&job.page_id], |r| {
                            r.get(0)
                        })?;
                    let current: Page = serde_json::from_str(&data)?;
                    let regions: Vec<_> = source_regions(&current, checkpoint)?
                        .into_iter()
                        .filter(|r| ids.contains(&r.id))
                        .collect();
                    ensure!(
                        ids.len() <= 6 && regions.len() == ids.len(),
                        "Glossary checkpoint regions changed"
                    );
                    let terms =
                        crate::glossary::validate_candidates(result.terms.clone(), &regions);
                    ensure!(terms == result.terms, "Glossary source evidence changed");
                    checkpoint.skipped += result.skipped;
                    if checkpoint.skipped > 0 {
                        checkpoint.warning = Some(format!(
                            "Skipped {} invalid glossary rows; valid terms were kept.",
                            checkpoint.skipped
                        ));
                    }
                    if pair_matches && book.glossary.enabled && book.glossary.auto_detect {
                        let mut changed = false;
                        for candidate in &terms {
                            // Current manually edited or earlier learned terminology always wins.
                            if book
                                .glossary
                                .entries
                                .iter()
                                .any(|e| e.source.trim() == candidate.source)
                            {
                                continue;
                            }
                            let entry = GlossaryEntry {
                                source: candidate.source.clone(),
                                target: candidate.target.clone(),
                            };
                            book.glossary.entries.push(entry.clone());
                            book.glossary.automatic_sources.push(entry.source.clone());
                            checkpoint.added += 1;
                            changed = true;
                            if !job
                                .settings
                                .glossary
                                .iter()
                                .any(|e| e.source == entry.source)
                            {
                                job.settings.glossary.push(entry);
                            }
                        }
                        crate::glossary::validate(&mut book.glossary)?;
                        if changed {
                            book.glossary.revision += 1;
                            book.revision += 1;
                            crate::library::put(&tx, &book)?;
                        }
                    } else {
                        checkpoint.warning = Some("Glossary changed or its language pair is different; detected terms were not saved".into());
                        checkpoint.completed = true;
                    }
                    checkpoint.processed.extend(ids);
                    tx.execute(
                        "INSERT OR REPLACE INTO cache VALUES(?1,?2)",
                        params![cache_key, serde_json::to_string(&result)?],
                    )?;
                }
                Update::Complete(warning) => {
                    checkpoint.completed = true;
                    if warning.is_some() {
                        checkpoint.warning = warning;
                    }
                }
                Update::Capture | Update::Refresh => unreachable!(),
            }
        }
        ensure!(!token.is_cancelled(), "Cancelled");
        tx.execute(
            "INSERT OR REPLACE INTO jobs VALUES(?1,?2)",
            params![job.id, crate::job_validation::serialize(&job)?],
        )?;
        tx.commit()?;
        let mut state = self.state.lock();
        let entry = state.jobs.get_mut(id).context("Job no longer exists")?;
        entry.requirements_key = crate::job_view::JobView::requirements_key(&job);
        entry.job = job.clone();
        let snapshot = state.touch(id);
        drop(state);
        (self.notify)(snapshot.clone());
        Ok(snapshot)
    }
    pub(super) async fn service_key(&self, provider: &ProviderProfile) -> Result<String> {
        let secret = self.secret.clone();
        let provider = provider.clone();
        tokio::task::spawn_blocking(move || {
            if providers::credential_required(&provider) {
                secret(&provider)
            } else {
                Ok(String::new())
            }
        })
        .await?
    }
    pub(super) async fn refresh_translation_glossary(
        self: &Arc<Self>,
        job: &mut Job,
        revision: u64,
    ) -> Result<()> {
        *job = self
            .glossary_update(&job.id, revision, Update::Refresh)
            .await?;
        Ok(())
    }

    pub(super) async fn prepare_glossary_sources(
        self: &Arc<Self>,
        job: &mut Job,
        page: &mut Page,
        expected: &mut u64,
        image: &image::DynamicImage,
        cancel: &CancellationToken,
    ) -> Result<()> {
        if let Some(checkpoint) = &mut job.glossary_checkpoint
            && checkpoint.eligible.is_none()
        {
            checkpoint.eligible = Some(
                page.regions
                    .iter()
                    .filter(|r| !crate::safety::has_text(&r.target))
                    .map(|r| r.id.clone())
                    .collect(),
            );
            self.checkpoint_with_glossary(
                &job.id,
                page,
                expected,
                "glossary",
                "running",
                "Checking source texts",
                job.glossary_checkpoint.clone(),
            )?;
        }
        let checkpoint = job
            .glossary_checkpoint
            .as_ref()
            .context("Missing glossary checkpoint")?;
        if job.settings.mode != "vision" || checkpoint.completed {
            return Ok(());
        }
        let eligible = checkpoint.eligible.as_deref().unwrap_or_default();
        let missing: Vec<_> = page
            .regions
            .iter()
            .filter(|r| eligible.contains(&r.id) && !crate::safety::has_text(&r.source))
            .cloned()
            .collect();
        if missing.is_empty() {
            self.step_result(
                &job.id,
                "transcribing",
                "skipped",
                "Using saved source text",
            );
        }
        for (index, batch) in missing.chunks(6).enumerate() {
            self.progress(
                &job.id,
                "transcribing",
                &format!("Batch {} / {}", index + 1, missing.len().div_ceil(6)),
            );
            let call = async {
                let key = self.service_key(&job.provider).await?;
                providers::transcribe_batch(&job.provider, &key, &job.settings, batch, image).await
            };
            let result = tokio::select! { _ = cancel.cancelled() => bail!("Cancelled"), result = call => result };
            ensure!(!cancel.is_cancelled(), "Cancelled");
            match result {
                Ok(items) => {
                    for item in &items {
                        if let Some(r) = page.regions.iter_mut().find(|r| r.id == item.id) {
                            r.source = item.source.clone();
                        }
                    }
                    self.checkpoint(
                        &job.id,
                        page,
                        expected,
                        "transcribing",
                        "complete",
                        "Recognized text saved",
                    )?;
                    if items.len() == batch.len() {
                        continue;
                    }
                    self.step_result(
                        &job.id,
                        "transcribing",
                        "warning",
                        "Incomplete source reading; continuing with existing terms",
                    );
                    *job = self.glossary_update(&job.id, *expected, Update::Complete(Some(
                        "Glossary detection unavailable; continuing with existing terms. Incomplete source reading".into()
                    ))).await?;
                    break;
                }
                Err(error) => {
                    self.step_result(&job.id, "transcribing", "warning", &error.to_string());
                    *job = self.glossary_update(&job.id, *expected, Update::Complete(Some(format!(
                        "Glossary detection unavailable; continuing with existing terms. {error}"
                    )))).await?;
                    break;
                }
            }
        }
        Ok(())
    }
    pub(super) async fn learn_glossary(
        self: &Arc<Self>,
        job: &mut Job,
        page: &Page,
        expected: &mut u64,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let id = job.id.clone();
        // Source-reading checkpoints own eligibility; extraction never needs targets.
        *job = self.job(&id).context("Job no longer exists")?;
        if !job
            .glossary_checkpoint
            .as_ref()
            .context("Missing glossary checkpoint")?
            .completed
        {
            let regions = source_regions(page, job.glossary_checkpoint.as_ref().unwrap())?;
            let processed = &job.glossary_checkpoint.as_ref().unwrap().processed;
            let regions: Vec<_> = regions
                .into_iter()
                .filter(|r| !processed.contains(&r.id))
                .collect();
            for (index, batch) in regions.chunks(6).enumerate() {
                if job.glossary_checkpoint.as_ref().unwrap().completed {
                    break;
                }
                self.progress(
                    &id,
                    "glossary",
                    &format!("Batch {} / {}", index + 1, regions.len().div_ceil(6)),
                );
                let cache_key = format!(
                    "glossary:validated-v3:{}",
                    store::digest(&serde_json::to_vec(&(
                        crate::prompts::GLOSSARY_PROMPT_VERSION,
                        crate::instructions::extraction_identity(&job.provider, &job.settings)?,
                        &job.settings.source_language,
                        &job.settings.target_language,
                        batch.iter().map(|r| (&r.id, &r.source)).collect::<Vec<_>>()
                    ))?)
                );
                let path = job.project.clone();
                let lookup = cache_key.clone();
                let expected_regions = batch.to_vec();
                let cached: Option<Extraction> = if job.fresh {
                    None
                } else {
                    tokio::task::spawn_blocking(move || -> Result<_> {
                        let c = store::connection(Path::new(&path))?;
                        let json: Option<String> = c
                            .query_row("SELECT data FROM cache WHERE key=?1", [lookup], |r| {
                                r.get(0)
                            })
                            .optional()?;
                        Ok(json.and_then(|v| {
                            crate::glossary::cached_candidates(&v, &expected_regions)
                        }))
                    })
                    .await??
                };
                let result = if let Some(result) = cached {
                    Ok(result)
                } else {
                    let call = async {
                        let key = self.service_key(&job.provider).await?;
                        providers::extract_glossary(&job.provider, &key, &job.settings, batch).await
                    };
                    tokio::select! { _ = cancel.cancelled() => bail!("Cancelled"), result = call => result }
                };
                ensure!(!cancel.is_cancelled(), "Cancelled");
                match result {
                    Ok(result) => {
                        *job = self
                            .glossary_update(
                                &id,
                                *expected,
                                Update::Extracted {
                                    ids: batch.iter().map(|r| r.id.clone()).collect(),
                                    result,
                                    cache_key,
                                },
                            )
                            .await?;
                    }
                    Err(error) => {
                        *job = self.glossary_update(&id, *expected, Update::Complete(Some(format!(
                            "Glossary detection unavailable; continuing with existing terms. {error}"
                        )))).await?;
                        break;
                    }
                }
            }
            *job = self
                .glossary_update(&id, *expected, Update::Complete(None))
                .await?;
        }
        let checkpoint = job.glossary_checkpoint.as_ref().unwrap();
        if let Some(warning) = &checkpoint.warning {
            self.step_result(&id, "glossary", "warning", warning);
        } else {
            self.step_result(
                &id,
                "glossary",
                "complete",
                &if checkpoint.added == 0 {
                    "No new terms".into()
                } else {
                    format!("{} terms added", checkpoint.added)
                },
            );
        }
        Ok(())
    }
}

/// Only accepted current sources authorize extraction, independently of translated text.
fn source_regions(page: &Page, checkpoint: &ExtractionCheckpoint) -> Result<Vec<Region>> {
    let eligible = checkpoint.eligible.as_deref().unwrap_or_default();
    ensure!(
        eligible
            .iter()
            .all(|id| page.regions.iter().any(|r| &r.id == id)),
        "Glossary source region is missing"
    );
    Ok(page
        .regions
        .iter()
        .filter(|r| eligible.contains(&r.id) && crate::safety::has_text(&r.source))
        .map(|r| {
            let mut source = r.clone();
            source.target.clear();
            source
        })
        .collect())
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    use crate::glossary::Candidate;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn update(kind: &str, page: &Page) -> Update {
        let id = page.regions[0].id.clone();
        match kind {
            "capture" => Update::Capture,
            "extracted" => Update::Extracted {
                ids: vec![id.clone()],
                result: Extraction {
                    terms: vec![Candidate {
                        region_id: id,
                        source: "Alice".into(),
                        target: "爱丽丝".into(),
                    }],
                    skipped: 0,
                },
                cache_key: "isolated-admission".into(),
            },
            "complete" => Update::Complete(None),
            _ => unreachable!(),
        }
    }

    #[test]
    fn every_glossary_checkpoint_rejects_oversized_jobs_without_publication() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        for kind in ["capture", "extracted", "complete"] {
            let dir = tempfile::tempdir_in(root.join("test-output")).unwrap();
            let source = dir.path().join("original.png");
            image::RgbImage::new(16, 16).save(&source).unwrap();
            let mut page = documents::import(&[source.to_string_lossy().into()])
                .unwrap()
                .remove(0);
            page.regions = vec![Region {
                bbox: [1., 1., 15., 15.],
                source: "Alice".into(),
                target: "爱丽丝".into(),
                ..Default::default()
            }];
            let project = store::create(
                &dir.path().join("book.umanga"),
                "Admission",
                &[page.clone()],
            )
            .unwrap();
            let book_path = Path::new(&project.path);
            let mut book = crate::library::open(book_path).unwrap();
            book.glossary.enabled = true;
            book.glossary.auto_detect = true;
            crate::library::save_glossary(
                dir.path(),
                book_path,
                book.glossary.revision,
                book.glossary,
            )
            .unwrap();
            let events = Arc::new(AtomicUsize::new(0));
            let observed = events.clone();
            let engine = Engine::new(
                root.join("assets/models"),
                &root.join("assets/fonts"),
                Arc::new(move |_| {
                    observed.fetch_add(1, Ordering::SeqCst);
                }),
                Arc::new(|_| bail!("No service calls")),
            )
            .unwrap();
            engine
                .enqueue_configured(
                    &project.path,
                    &[page.id.clone()],
                    &ProviderProfile::default(),
                    &TranslationSettings {
                        auto_glossary: true,
                        glossary_enabled: true,
                        ..Default::default()
                    },
                )
                .unwrap();
            let (job, _) = engine.pick().unwrap();
            let path = Path::new(&project.path);
            let c = store::connection(path).unwrap();
            if kind == "extracted" {
                store::cache_put_validated(
                    path,
                    "translation",
                    &[TranslationItem {
                        id: page.regions[0].id.clone(),
                        source: "Alice".into(),
                        target: "爱丽丝".into(),
                        direction: String::new(),
                    }],
                )
                .unwrap();
            }
            let persisted_job: String = c
                .query_row("SELECT data FROM jobs WHERE id=?1", [&job.id], |r| r.get(0))
                .unwrap();
            let persisted_page: String = c
                .query_row("SELECT data FROM pages WHERE id=?1", [&page.id], |r| {
                    r.get(0)
                })
                .unwrap();
            let persisted_book: String = c
                .query_row("SELECT value FROM meta WHERE key='book'", [], |r| r.get(0))
                .unwrap();
            let before_events = events.load(Ordering::SeqCst);
            let oversized = {
                let mut state = engine.state.lock();
                let entry = state.jobs.get_mut(&job.id).unwrap();
                entry.job.error = Some("x".repeat(3 * 1024 * 1024));
                entry.job.steps = vec![JobStep {
                    stage: "translating".into(),
                    status: "failed".into(),
                    detail: String::new(),
                    error: Some("x".repeat(3 * 1024 * 1024)),
                }];
                if kind != "capture" {
                    entry.job.glossary_checkpoint = Some(ExtractionCheckpoint {
                        eligible: Some(vec![page.regions[0].id.clone()]),
                        ..Default::default()
                    });
                }
                crate::job_validation::validate(
                    &job.id,
                    &entry.job,
                    page.revision,
                    &page.regions.iter().map(|r| r.id.clone()).collect(),
                )
                .unwrap();
                serde_json::to_string(&entry.job).unwrap()
            };
            let error = engine
                .glossary_update_sync(&job.id, page.revision, update(kind, &page))
                .unwrap_err();
            assert!(error.to_string().contains("size limit"), "{kind}: {error}");
            assert_eq!(
                c.query_row("SELECT data FROM jobs WHERE id=?1", [&job.id], |r| r
                    .get::<_, String>(0))
                    .unwrap(),
                persisted_job,
                "{kind}"
            );
            assert_eq!(
                c.query_row("SELECT data FROM pages WHERE id=?1", [&page.id], |r| r
                    .get::<_, String>(
                    0
                ))
                .unwrap(),
                persisted_page,
                "{kind}"
            );
            assert_eq!(
                c.query_row("SELECT value FROM meta WHERE key='book'", [], |r| r
                    .get::<_, String>(0))
                    .unwrap(),
                persisted_book,
                "{kind}"
            );
            assert_eq!(
                c.query_row("SELECT count(*) FROM cache", [], |r| r.get::<_, usize>(0))
                    .unwrap(),
                usize::from(kind == "extracted"),
                "{kind}"
            );
            assert_eq!(events.load(Ordering::SeqCst), before_events, "{kind}");
            assert_eq!(
                serde_json::to_string(&engine.state.lock().jobs[&job.id].job).unwrap(),
                oversized,
                "{kind}"
            );
            {
                let mut state = engine.state.lock();
                let entry = state.jobs.get_mut(&job.id).unwrap();
                entry.job.error = None;
                entry.job.steps.clear();
            }
            let saved = engine
                .glossary_update_sync(&job.id, page.revision, update(kind, &page))
                .unwrap();
            assert_eq!(events.load(Ordering::SeqCst), before_events + 1, "{kind}");
            let recovered = store::jobs(path).unwrap().remove(0);
            assert_eq!(recovered.id, saved.id);
            if kind == "extracted" {
                assert_eq!(
                    crate::library::open(path).unwrap().glossary.entries[0].source,
                    "Alice"
                );
                assert_eq!(
                    c.query_row("SELECT count(*) FROM cache", [], |r| r.get::<_, usize>(0))
                        .unwrap(),
                    2
                );
            }
            if kind == "complete" {
                assert!(saved.glossary_checkpoint.unwrap().completed);
            }
        }
    }
}
