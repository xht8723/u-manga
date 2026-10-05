//! Session-only region work. No page/cache writes and no detector or renderer calls.
use super::*;
use anyhow::ensure;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RegionAction {
    Read,
    Translate,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionRequest {
    pub id: String,
    pub path: String,
    pub page_id: String,
    pub expected: u64,
    pub region: Region,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionResult {
    pub id: String,
    pub page_id: String,
    pub region_id: String,
    pub expected: u64,
    pub text: String,
    pub elapsed_ms: u64,
}
impl Engine {
    // Keep the captured configuration, reservation and progress sink explicit at this boundary.
    #[allow(clippy::too_many_arguments)]
    pub async fn process_region(
        self: &Arc<Self>,
        request: RegionRequest,
        action: RegionAction,
        settings: TranslationSettings,
        provider: ProviderProfile,
        directory: PathBuf,
        guard: scheduler::RegionGuard,
        progress: Arc<dyn Fn(&str) + Send + Sync>,
    ) -> Result<RegionResult> {
        let provider = crate::instructions::capture(&provider, &settings)?;
        let started = Instant::now();
        let cancel = guard.cancel.clone();
        ensure!(!cancel.is_cancelled(), "Cancelled");
        crate::safety::region(&request.region)?;
        ensure!(
            !request.region.id.is_empty() && request.region.id.len() <= 256,
            "Invalid region identity"
        );
        progress("loading");
        let path = request.path.clone();
        let page_id = request.page_id.clone();
        let expected = request.expected;
        let need_image = action == RegionAction::Read;
        let (page, image) = tokio::task::spawn_blocking(move || -> Result<_> {
            let page = store::page(Path::new(&path), &page_id)?;
            ensure!(
                page.revision == expected,
                "Page changed; retry the region action"
            );
            let im = if need_image {
                documents::source_health(&page)?;
                let im = documents::load(&page.source)?;
                ensure!(
                    page.fingerprint.is_empty()
                        || page.fingerprint == store::digest(im.to_rgb8().as_raw()),
                    "Original file changed; restore it before rereading"
                );
                Some(im)
            } else {
                None
            };
            Ok((page, im))
        })
        .await??;
        let r = &request.region;
        ensure!(
            r.bbox[0] >= 0.
                && r.bbox[1] >= 0.
                && r.bbox[2] <= page.width as f32
                && r.bbox[3] <= page.height as f32
                && r.bbox[2] > r.bbox[0]
                && r.bbox[3] > r.bbox[1],
            "Region must be inside the page"
        );
        ensure!(!cancel.is_cancelled(), "Cancelled");
        let text = if action == RegionAction::Read && settings.mode == "local" {
            progress("ocr");
            let engine = self.clone();
            let bbox = r.bbox;
            let settings = settings.clone();
            let token = cancel.clone();
            // Await worker exit even after cancellation; its page/model reservation stays alive.
            tokio::task::spawn_blocking(move || -> Result<String> {
                let mut inference = engine.inference_for(&token)?;
                ensure!(!token.is_cancelled(), "Cancelled");
                inference.set_root(directory);
                let texts = inference.recognize_cancellable(
                    &[inference::crop(image.as_ref().unwrap(), bbox)],
                    &settings,
                    &token,
                )?;
                ensure!(texts.len() == 1, "OCR returned the wrong number of regions");
                Ok(texts[0].clone())
            })
            .await??
        } else {
            if action == RegionAction::Translate {
                ensure!(
                    !r.source.trim().is_empty(),
                    "Enter source text before translating this region"
                );
                ensure!(
                    !crate::instructions::simple_translation(&provider, false)
                        || crate::instructions::simple_source_readable(&r.source),
                    "Source contains only unreadable redaction blocks; correct the source before translating"
                );
            }
            progress(if action == RegionAction::Read {
                "transcribing"
            } else {
                "translating"
            });
            let source_context = if action == RegionAction::Translate {
                let path = request.path.clone();
                let count = settings.context_pages;
                tokio::task::spawn_blocking(move || {
                    store::preceding_context(Path::new(&path), page.number, count)
                })
                .await??
            } else {
                crate::prompts::SourceContext::default()
            };
            let worker = self.clone();
            let p = provider.clone();
            let secret = tokio::task::spawn_blocking(move || {
                if providers::credential_required(&p) {
                    (worker.secret)(&p)
                } else {
                    Ok(String::new())
                }
            })
            .await??;
            let mut crop_region = r.clone();
            let attachments = if action == RegionAction::Read {
                crop_region.source.clear();
                crop_region.target.clear();
                // Await encoding exit without a cancellation select: the book remains owned.
                Some(
                    providers::prepare_vision_region(
                        image.unwrap(),
                        crop_region.clone(),
                        cancel.clone(),
                    )
                    .await?,
                )
            } else {
                None
            };
            ensure!(!cancel.is_cancelled(), "Cancelled");
            let call = async {
                if action == RegionAction::Read {
                    providers::transcribe_prepared(
                        &provider,
                        &secret,
                        &settings,
                        &crop_region,
                        attachments.as_ref().unwrap(),
                    )
                    .await
                } else {
                    let items = if provider.service == "llm" {
                        providers::llm(
                            &provider,
                            &secret,
                            &settings,
                            std::slice::from_ref(r),
                            None,
                            &source_context.for_llm(),
                        )
                        .await?
                    } else {
                        providers::conventional(
                            &provider,
                            &secret,
                            &settings,
                            std::slice::from_ref(r),
                            &source_context.for_service(),
                        )
                        .await?
                    };
                    Ok(items
                        .into_iter()
                        .find(|i| i.id == r.id)
                        .context(
                            "No translation returned for this region; existing text preserved",
                        )?
                        .target)
                }
            };
            tokio::select! { _ = cancel.cancelled() => bail!("Cancelled"), result = call => result? }
        };
        let text = if action == RegionAction::Translate {
            let formatted = crate::render::format_translation(&request.region, &text);
            ensure!(
                crate::safety::has_text(&formatted),
                "Translation has no usable lettering text; edit the translation to continue."
            );
            crate::safety::response_text(&request.region.source, &formatted)?;
            formatted
        } else {
            text
        };
        ensure!(!cancel.is_cancelled(), "Cancelled");
        let path = request.path.clone();
        let id = request.page_id.clone();
        ensure!(
            tokio::task::spawn_blocking(move || store::page(Path::new(&path), &id))
                .await??
                .revision
                == expected,
            "Page changed; region result discarded"
        );
        Ok(RegionResult {
            id: request.id,
            page_id: request.page_id,
            region_id: request.region.id,
            expected,
            text,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }
}
