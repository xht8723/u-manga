use crate::{
    documents,
    inference::{self, Inference},
    providers,
    render::Renderer,
    store,
    types::*,
};
use anyhow::{Context, Result, bail};
use parking_lot::Mutex;
use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
use tokio_util::sync::CancellationToken;
type Notify = Arc<dyn Fn(Job) + Send + Sync>;
type Secret = Arc<dyn Fn(&ProviderProfile) -> Result<String> + Send + Sync>;
#[path = "auto_glossary.rs"]
mod auto_glossary;
#[path = "batch_translation.rs"]
mod batch_translation;
#[path = "region_requests.rs"]
mod region_requests;
#[path = "scheduler.rs"]
mod scheduler;
pub use batch_translation::{
    BatchMode, BatchOutcome, BatchPage, BatchPreview, BatchSubmission, PageVersion,
};
pub use region_requests::{RegionAction, RegionRequest, RegionResult};
pub use scheduler::{
    BookReservation, ControlResult, Engine, JobCheck, JobFailure, PageEditGuard, RegionGuard,
    RuntimeJob,
};
#[cfg(test)]
#[path = "preparation_tests.rs"]
mod preparation_tests;
impl Engine {
    async fn process(self: Arc<Self>, id: &str, snapshot: (Job, CancellationToken)) -> Result<()> {
        let (mut job, cancel) = snapshot;
        if cancel.is_cancelled() {
            bail!("Cancelled");
        }
        if let Err(error) = self.check_requirements(job.clone()).await {
            self.block_job(id, &format!("{error:#}"));
            bail!("{error:#}");
        }
        let path = PathBuf::from(&job.project);
        let _outputs = crate::assets::PageOutputs::new(&path, &job.page_id)?;
        self.progress(id, "loading", "");
        let mut page = store::page(&path, &job.page_id)?;
        let preparing = job.kind == JobKind::Preparation;
        if job.fresh && job.page_revision != Some(page.revision) {
            bail!("Page changed; review it and confirm preparation again");
        }
        let mut expected = page.revision;
        self.capture_glossary(&mut job, expected).await?;
        documents::source_health(&page)?;
        let source = page.source.clone();
        let (im, hash) = tokio::task::spawn_blocking(move || -> Result<_> {
            let im = documents::load(&source)?;
            let hash = store::digest(im.to_rgb8().as_raw());
            Ok((im, hash))
        })
        .await??;
        if !page.fingerprint.is_empty() && page.fingerprint != hash {
            bail!(
                "Original file changed. Restore the matching original or add it as a new page before translating"
            )
        };
        page.fingerprint = hash;
        if cancel.is_cancelled() {
            bail!("Cancelled")
        }
        if job.kind != JobKind::Cleanup {
            let fresh_detection = job.fresh
                && !job.steps.iter().any(|s| {
                    s.stage == "detecting" && ["complete", "skipped"].contains(&s.status.as_str())
                });
            if fresh_detection || (!job.fresh && page.regions.is_empty()) {
                self.progress(id, "detecting", "");
                let worker = self.clone();
                let image = im.clone();
                let detection_cancel = cancel.clone();
                let detected = tokio::task::spawn_blocking(move || {
                    worker
                        .inference_for(&detection_cancel)?
                        .detect_cancellable(&image, &detection_cancel)
                })
                .await??;
                if cancel.is_cancelled() {
                    bail!("Cancelled")
                };
                replace_detected(&mut page, detected, job.fresh, preparing);
                let detail = format!("{} regions saved", page.regions.len());
                self.checkpoint(
                    id,
                    &mut page,
                    &mut expected,
                    "detecting",
                    "complete",
                    &detail,
                )?;
            } else {
                self.step_result(id, "detecting", "skipped", "Using saved regions");
            }
            let prepared_ocr = job.fresh
                && job.steps.iter().any(|s| {
                    s.stage == "ocr" && ["complete", "skipped"].contains(&s.status.as_str())
                });
            if job.settings.mode == "local"
                && !prepared_ocr
                && page
                    .regions
                    .iter()
                    .any(|r| !crate::safety::has_text(&r.source))
            {
                self.progress(id, "ocr", "");
                if job.settings.ocr == "manga" && job.settings.source_language != "ja" {
                    bail!("Manga OCR supports Japanese; choose PP-OCR for this source language")
                };
                let worker = self.clone();
                let image = im.clone();
                let regions: Vec<_> = page
                    .regions
                    .iter()
                    .filter(|r| !crate::safety::has_text(&r.source))
                    .cloned()
                    .collect();
                let region_ids: Vec<_> = regions.iter().map(|r| r.id.clone()).collect();
                let settings = job.settings.clone();
                let model_directory = PathBuf::from(&job.model_directory);
                let ocr_cancel = cancel.clone();
                let (texts, device) =
                    tokio::task::spawn_blocking(move || -> Result<(Vec<String>, String)> {
                        let mut infer = worker.inference_for(&ocr_cancel)?;
                        infer.set_root(model_directory);
                        let boxes = regions.iter().map(|r| r.bbox).collect::<Vec<_>>();
                        let text = infer.recognize_regions_cancellable(
                            &image,
                            &boxes,
                            &settings,
                            &ocr_cancel,
                        )?;
                        Ok((text, infer.device_message.clone()))
                    })
                    .await??;
                if cancel.is_cancelled() {
                    bail!("Cancelled")
                };
                self.progress(id, "ocr", &device);
                anyhow::ensure!(
                    texts.len() == region_ids.len(),
                    "OCR returned the wrong number of regions"
                );
                for (id, text) in region_ids.into_iter().zip(texts) {
                    if let Some(r) = page
                        .regions
                        .iter_mut()
                        .find(|r| r.id == id && !crate::safety::has_text(&r.source))
                    {
                        r.source = text;
                    }
                }
                let detail = format!(
                    "{} recognized regions saved · {device}",
                    page.regions
                        .iter()
                        .filter(|r| crate::safety::has_text(&r.source))
                        .count()
                );
                self.checkpoint(id, &mut page, &mut expected, "ocr", "complete", &detail)?;
            } else if job.settings.mode == "local" {
                self.step_result(id, "ocr", "skipped", "Using saved source text");
            }
        }
        if job.kind == JobKind::Translation {
            if job.detects_glossary() {
                self.prepare_glossary_sources(&mut job, &mut page, &mut expected, &im, &cancel)
                    .await?;
                self.learn_glossary(&mut job, &page, &mut expected, &cancel)
                    .await?;
            }
            self.refresh_translation_glossary(&mut job, expected)
                .await?;
            let source_context =
                store::preceding_context(&path, page.number, job.settings.context_pages)?;
            let context = source_context.for_llm();
            let service_context = source_context.for_service();
            let context_key = source_context.cache_identity()?;
            self.progress(id, "translating", "Checking cached translations");
            let key = request_cache_key(
                store::cache_key(&page, &job.settings, &job.provider, &context_key)?,
                &job,
            );
            let text_only = job.settings.mode == "local";
            let simple = crate::instructions::simple_translation(&job.provider, !text_only);
            let whole_page_text = text_only && job.provider.service == "llm";
            // Text context depends on exact request membership. Reuse only request-level
            // caches for these routes, not an aggregate assembled from different batches.
            let page_cache = if whole_page_text {
                None
            } else {
                store::cache_validated(&path, &key, &page.regions, text_only)?.filter(|items| {
                    page.regions.iter().all(|r| {
                        crate::safety::has_text(&r.target) || items.iter().any(|i| i.id == r.id)
                    })
                })
            };
            let translations = if let Some(cached) = page_cache {
                store::cache_put_validated(&path, &key, &cached)?;
                cached
            } else {
                let mut translations = Vec::new();
                let pending = page
                    .regions
                    .iter()
                    .filter(|r| {
                        !crate::safety::has_text(&r.target)
                            && (!text_only || crate::safety::has_text(&r.source))
                            && (!simple || crate::instructions::simple_source_readable(&r.source))
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                // Image/conventional requests keep their original six-region batching.
                let batches = if whole_page_text {
                    crate::text_batches::plan(&pending, simple)?
                } else {
                    pending.chunks(6).collect()
                };
                for (batch_index, batch) in batches.iter().copied().enumerate() {
                    if cancel.is_cancelled() {
                        bail!("Cancelled")
                    };
                    let mut partial = page.clone();
                    partial.regions = batch.to_vec();
                    let request_key = request_cache_key(
                        store::cache_key(&partial, &job.settings, &job.provider, &context_key)?,
                        &job,
                    );
                    let mut cached = store::cache_validated(&path, &request_key, batch, text_only)?
                        .unwrap_or_default();
                    if whole_page_text && !batch.iter().all(|r| cached.iter().any(|t| t.id == r.id))
                    {
                        // Never shrink a text request around partial cache hits: its neighbouring
                        // originals are context. Successful durable page checkpoints still survive.
                        cached.clear();
                    }
                    let remaining = batch
                        .iter()
                        .filter(|r| !cached.iter().any(|t| t.id == r.id))
                        .cloned()
                        .collect::<Vec<_>>();
                    if remaining.is_empty() {
                        store::cache_put_validated(&path, &request_key, &cached)?;
                        apply_translations(&mut page, &cached);
                        let detail = translation_detail(&page);
                        self.translation_checkpoint(
                            &job,
                            &mut page,
                            &mut expected,
                            "running",
                            &detail,
                        )?;
                        translations.extend(cached);
                        continue;
                    }
                    let batch = remaining.as_slice();
                    self.progress(
                        id,
                        "translating",
                        &format!(
                            "Batch {} / {} · {} {}",
                            batch_index + 1,
                            batches.len(),
                            batch.len(),
                            if batch.len() == 1 {
                                "region"
                            } else {
                                "regions"
                            }
                        ),
                    );
                    let secret = if providers::credential_required(&job.provider) {
                        (self.secret)(&job.provider)?
                    } else {
                        String::new()
                    };
                    let call = async {
                        if job.provider.service == "llm" {
                            providers::llm(
                                &job.provider,
                                &secret,
                                &job.settings,
                                batch,
                                if job.settings.mode == "vision" {
                                    Some(&im)
                                } else {
                                    None
                                },
                                &context,
                            )
                            .await
                        } else {
                            providers::conventional(
                                &job.provider,
                                &secret,
                                &job.settings,
                                batch,
                                &service_context,
                            )
                            .await
                        }
                    };
                    let items =
                        tokio::select! {_ = cancel.cancelled()=>bail!("Cancelled"),r=call=>r?};
                    if cancel.is_cancelled() {
                        bail!("Cancelled");
                    }
                    cached.extend(items);
                    store::cache_put_validated(&path, &request_key, &cached)?;
                    apply_translations(&mut page, &cached);
                    let detail = translation_detail(&page);
                    self.translation_checkpoint(
                        &job,
                        &mut page,
                        &mut expected,
                        "running",
                        &detail,
                    )?;
                    translations.extend(cached);
                }
                if !whole_page_text {
                    store::cache_put_validated(&path, &key, &translations)?;
                }
                translations
            };
            if cancel.is_cancelled() {
                bail!("Cancelled")
            };
            apply_translations(&mut page, &translations);
            for r in &mut page.regions {
                if !crate::safety::has_text(&r.target) {
                    r.review = Some("Missing or unreadable translation; original preserved".into())
                }
            }
            let missing = page
                .regions
                .iter()
                .filter(|r| !crate::safety::has_text(&r.target))
                .count();
            let detail = if missing > 0 {
                format!(
                    "{} · {missing} missing; originals preserved",
                    translation_detail(&page)
                )
            } else {
                translation_detail(&page)
            };
            self.translation_checkpoint(
                &job,
                &mut page,
                &mut expected,
                if missing > 0 { "warning" } else { "complete" },
                &detail,
            )?;
        }

        let worker = self.clone();
        let mut draft = page.clone();
        let mut settings = job.settings.clone();
        let reclean = job.kind == JobKind::Cleanup || (job.fresh && page.cleanup.is_none());
        if !reclean && let Some(saved) = &page.cleanup {
            settings.cleanup = saved.settings.clone();
        }
        self.progress(id, "cleaning", settings.cleanup.method.name());
        let book_path = path.clone();
        let directory = PathBuf::from(&job.model_directory);
        let token = cancel.clone();
        let event_engine = self.clone();
        let event_id = id.to_string();
        let method = settings.cleanup.method;
        let model = method.name().to_string();
        let progress = Arc::new(move |device: &str, n: usize, total: usize| {
            event_engine.cleanup_progress(
                &event_id,
                if total == 0 && !preparing {
                    "lettering"
                } else {
                    "cleaning"
                },
                device,
                &format!(
                    "{model} · {device}{}",
                    if total == 0 {
                        String::new()
                    } else {
                        format!(" · tile {n}/{total}")
                    }
                ),
                method,
            );
        });
        let output = tokio::task::spawn_blocking(move || -> Result<_> {
            let rendered = worker.render_page_checkpointed(
                &im,
                &mut draft,
                &settings,
                &book_path,
                &directory,
                reclean,
                &token,
                progress,
                |page| {
                    let issues = page.cleanup.as_ref().map(|c| c.reviews.len()).unwrap_or(0);
                    let detail = if issues > 0 {
                        page.cleanup
                            .as_ref()
                            .and_then(|c| c.reviews.values().next())
                            .cloned()
                            .unwrap_or_default()
                    } else {
                        "Cleaned background saved".into()
                    };
                    worker.checkpoint(
                        &job.id,
                        page,
                        &mut expected,
                        "cleaning",
                        if issues > 0 { "warning" } else { "complete" },
                        &detail,
                    )
                },
            )?;
            Ok((draft, rendered))
        })
        .await??;
        page = output.0;
        expected = page.revision;
        if cancel.is_cancelled() {
            bail!("Cancelled");
        }
        let issues = page
            .regions
            .iter()
            .filter(|r| {
                crate::safety::has_text(&r.target)
                    && r.review.is_some()
                    && !page
                        .cleanup
                        .as_ref()
                        .is_some_and(|c| c.reviews.contains_key(&r.id))
            })
            .count();
        if !preparing {
            self.step_result(
                id,
                "lettering",
                if issues > 0 { "warning" } else { "complete" },
                &if issues > 0 {
                    format!("{issues} regions need review; select their boxes for details")
                } else {
                    "Lettering ready".into()
                },
            );
        }
        self.progress(id, "saving", "");
        let asset = crate::safety::page_asset(&path, &page.id, &format!("{}.png", uid()))?;
        crate::assets::save_image(&output.1, &asset)?;
        page.rendered = Some(asset.to_string_lossy().into());
        page.status = if page.regions.iter().any(|r| r.review.is_some()) {
            "review"
        } else if page
            .regions
            .iter()
            .any(|r| r.prepared && !crate::safety::has_text(&r.target))
        {
            "prepared"
        } else {
            "complete"
        }
        .into();
        if cancel.is_cancelled() {
            bail!("Cancelled")
        };
        self.checkpoint(
            id,
            &mut page,
            &mut expected,
            "saving",
            "complete",
            "Rendered page saved",
        )?;
        Ok(())
    }
}

impl Engine {
    fn translation_checkpoint(
        &self,
        job: &Job,
        page: &mut Page,
        expected: &mut u64,
        status: &str,
        detail: &str,
    ) -> Result<()> {
        self.checkpoint_with_glossary(
            &job.id,
            page,
            expected,
            "translating",
            status,
            detail,
            job.glossary_checkpoint.clone(),
        )
    }
}
fn apply_translations(page: &mut Page, translations: &[TranslationItem]) {
    for r in &mut page.regions {
        if let Some(t) = translations.iter().find(|t| t.id == r.id)
            && !crate::safety::has_text(&r.target)
        {
            if !crate::safety::has_text(&r.source) {
                r.source = t.source.clone();
            }
            let target = crate::render::format_translation(r, &t.target);
            if !crate::safety::has_text(&target)
                || crate::safety::response_text(&r.source, &target).is_err()
            {
                // A failed generated translation cannot authorize erasing the original.
                r.prepared = false;
                r.review = Some(
                    "Translation has no usable lettering text; edit the translation to continue."
                        .into(),
                );
                continue;
            }
            r.target = target;
            r.review = None;
            // Direction remains a local/editor decision, never a provider value.
            page.rendered = None;
            page.status = "processing".into();
        }
    }
}
fn translation_detail(page: &Page) -> String {
    format!(
        "{} / {} translations saved",
        page.regions
            .iter()
            .filter(|r| crate::safety::has_text(&r.target))
            .count(),
        page.regions.len()
    )
}

// A fresh run bypasses older requests but can resume its own completed requests.
fn request_cache_key(key: String, job: &Job) -> String {
    if job.fresh {
        format!("{key}:fresh:{}", job.id)
    } else {
        key
    }
}

fn replace_detected(page: &mut Page, regions: Vec<Region>, fresh: bool, preparing: bool) {
    page.regions = regions;
    page.rendered = None;
    if fresh {
        page.background = None;
        page.cleanup = None;
        page.error = None;
        for r in &mut page.regions {
            r.prepared = preparing;
        }
    }
    page.status = "processing".into();
}

#[cfg(test)]
mod acceptance_tests {
    use super::*;

    fn page(prepared: bool) -> Page {
        let temp = tempfile::tempdir().unwrap();
        let original = temp.path().join("source.png");
        image::RgbImage::new(16, 16).save(&original).unwrap();
        let mut page = documents::import(&[original.to_string_lossy().into()])
            .unwrap()
            .remove(0);
        page.regions = vec![Region {
            id: "r".into(),
            prepared,
            ..Default::default()
        }];
        page
    }

    #[test]
    fn new_targets_are_formatted_while_manual_targets_and_raw_replies_survive() {
        let mut page = page(false);
        let reply = TranslationItem {
            id: "r".into(),
            source: "source".into(),
            target: "阿·布=朋友！你好，世界。".into(),
            direction: String::new(),
        };
        apply_translations(&mut page, std::slice::from_ref(&reply));
        assert_eq!(page.regions[0].target, "阿·布=朋友！\n你好\n世界");
        assert_eq!(reply.target, "阿·布=朋友！你好，世界。");
        page.regions[0].target = "手工，句号。·=！\n\n换行".into();
        apply_translations(&mut page, &[reply]);
        assert_eq!(page.regions[0].target, "手工，句号。·=！\n\n换行");
    }

    #[test]
    fn unusable_generated_text_revokes_prepared_cleanup_and_reports_review() {
        let mut page = page(true);
        apply_translations(
            &mut page,
            &[TranslationItem {
                id: "r".into(),
                source: "source".into(),
                target: "，。".into(),
                direction: String::new(),
            }],
        );
        assert!(page.regions[0].target.is_empty());
        assert!(!page.regions[0].prepared);
        assert!(
            page.regions[0]
                .review
                .as_deref()
                .unwrap()
                .contains("no usable lettering text")
        );
    }
}
