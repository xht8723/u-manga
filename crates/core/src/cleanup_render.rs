//! Cleanup assets are independent of lettering and of the translation cache.
use crate::{cleanup, inpainting, pipeline::Engine, store, types::*};
use anyhow::{Result, ensure};
use image::{DynamicImage, GrayImage, Luma, Rgb, RgbImage};
use std::{collections::BTreeMap, path::Path};
use tokio_util::sync::CancellationToken;

impl Engine {
    #[allow(
        clippy::too_many_arguments,
        reason = "Explicit processing inputs preserve the shared production/benchmark and captured-job contracts"
    )]
    pub fn render_page(
        &self,
        original: &DynamicImage,
        page: &mut Page,
        settings: &TranslationSettings,
        book: &Path,
        directory: &Path,
        force: bool,
        cancel: &CancellationToken,
        progress: inpainting::Progress,
    ) -> Result<DynamicImage> {
        self.render_page_checkpointed(
            original,
            page,
            settings,
            book,
            directory,
            force,
            cancel,
            progress,
            |_| Ok(()),
        )
    }
    #[allow(
        clippy::too_many_arguments,
        reason = "Explicit processing inputs preserve the shared production/benchmark and captured-job contracts"
    )]
    pub fn render_page_checkpointed(
        &self,
        original: &DynamicImage,
        page: &mut Page,
        settings: &TranslationSettings,
        book: &Path,
        directory: &Path,
        force: bool,
        cancel: &CancellationToken,
        progress: inpainting::Progress,
        mut checkpoint: impl FnMut(&mut Page) -> Result<()>,
    ) -> Result<DynamicImage> {
        ensure!(!cancel.is_cancelled(), "Cancelled");
        let (_, _assets) =
            crate::assets::protect_snapshot(|| Ok(((), crate::assets::page_references(page))))?;
        crate::safety::render_budget(page)?;
        ensure!(
            crate::documents::matches_dimensions(page, original.dimensions()),
            "Original dimensions changed"
        );
        ensure!(
            original.width() as u64 * original.height() as u64 <= 100_000_000,
            "Image exceeds render budget"
        );
        let recipe = if force {
            settings.cleanup.clone()
        } else {
            page.cleanup
                .as_ref()
                .map(|c| c.settings.clone())
                .unwrap_or_else(|| settings.cleanup.clone())
        };
        let rgb = original.to_rgb8();
        let (w, h) = rgb.dimensions();
        let imported = page
            .background
            .as_ref()
            .map(|path| crate::image_input::open(Path::new(path)))
            .transpose()?;
        if let Some(im) = &imported {
            ensure!(
                im.dimensions() == (w, h),
                "Cleaned background dimensions must match the original"
            );
        }
        let mut base = imported
            .as_ref()
            .map(DynamicImage::to_rgb8)
            .unwrap_or_else(|| rgb.clone());
        let mut full = GrayImage::new(w, h);
        let mut neural = full.clone();
        let mut protected = full.clone();
        let bubbles: Vec<_> = page.regions.iter().filter_map(|r| r.bubble).collect();
        let mut letters = crate::lettering_spool::Spool::new(book, &page.id)?;
        let mut masks = vec![];
        let mut neural_ids = vec![];
        let mut reviews = BTreeMap::new();
        // Lettering failures are evaluated on every edit, independently of cached cleanup.
        let mut cleanup_reviews = BTreeMap::new();
        for (index, r) in page.regions.iter_mut().enumerate() {
            ensure!(!cancel.is_cancelled(), "Cancelled");
            if !crate::safety::has_text(&r.target) && !r.prepared {
                r.review
                    .get_or_insert_with(|| "Missing translation; original preserved".into());
                continue;
            }
            r.review = None;
            let analyzed = if imported.is_none() && !r.overlay_only {
                Some(cleanup::analyze_with_bubbles(&rgb, r, &bubbles))
            } else {
                None
            };
            if crate::safety::has_text(&r.target) {
                match self.renderer.lettering(r, &settings.target_language) {
                    Ok(Some(pixmap)) => letters.push(index, pixmap)?,
                    Ok(None) => {
                        reviews.insert(
                            r.id.clone(),
                            "No lettering remains after formatting; original preserved".into(),
                        );
                        if let Some(c) = &analyzed {
                            stamp(&mut protected, c);
                        }
                        continue;
                    }
                    Err(e) => {
                        reviews.insert(r.id.clone(), e.to_string());
                        if let Some(c) = &analyzed {
                            stamp(&mut protected, c);
                        }
                        continue;
                    }
                }
            }
            if let Some(c) = analyzed {
                let use_model = recipe.method != CleanupMethod::Solid
                    && !r.allow_fill
                    && (recipe.strategy == CleanupStrategy::AllRegions || c.interior.is_none());
                if use_model && c.mask.as_raw().iter().any(|v| *v != 0) {
                    neural_ids.push(r.id.clone());
                }
                masks.push((c, use_model));
            }
        }
        // Missing translations remain intact, including intersections with another region.
        if imported.is_none() {
            for r in &page.regions {
                if !crate::safety::has_text(&r.target) && !r.prepared {
                    stamp(
                        &mut protected,
                        &cleanup::analyze_with_bubbles(&rgb, r, &bubbles),
                    );
                }
            }
        }
        for (c, use_model) in &masks {
            for (x, y, m) in c.mask.enumerate_pixels() {
                let x = x + c.origin[0];
                let y = y + c.origin[1];
                if m[0] > 0 && x < w && y < h && protected.get_pixel(x, y)[0] == 0 {
                    full.put_pixel(x, y, Luma([255]));
                    if *use_model {
                        neural.put_pixel(x, y, Luma([255]));
                    } else {
                        base.put_pixel(x, y, Rgb(c.fill));
                    }
                }
            }
        }
        // Neural and explicitly approved solid masks can overlap: manual fill wins.
        for (c, use_model) in &masks {
            if !*use_model {
                for (x, y, m) in c.mask.enumerate_pixels() {
                    let x = x + c.origin[0];
                    let y = y + c.origin[1];
                    if m[0] > 0 && x < w && y < h {
                        neural.put_pixel(x, y, Luma([0]));
                    }
                }
            }
        }
        let checksum = recipe
            .method
            .pack()
            .and_then(|id| {
                crate::models::catalog()
                    .ok()?
                    .into_iter()
                    .find(|p| p.id == id)
            })
            .map(|p| p.files[0].sha256.clone());
        let key = store::digest(&serde_json::to_vec(&(
            inpainting::ALGORITHM_VERSION,
            &recipe,
            checksum,
            page.regions
                .iter()
                .map(|r| (&r.id, r.bbox, r.overlay_only, r.allow_fill, r.prepared))
                .collect::<Vec<_>>(),
            store::digest(rgb.as_raw()),
            store::digest(full.as_raw()),
            store::digest(neural.as_raw()),
            store::digest(base.as_raw()),
        ))?);
        let incomplete = page.cleanup.as_ref().is_some_and(|c| !c.reviews.is_empty());
        let cached = if !force {
            page.cleanup
                .as_ref()
                .filter(|c| c.key == key && c.reviews.is_empty())
                .and_then(|c| {
                    crate::image_input::open(Path::new(&c.path))
                        .ok()
                        .filter(|im| im.dimensions() == (w, h))
                        .map(|im| {
                            (
                                im.to_rgb8(),
                                c.device.clone(),
                                c.reviews.clone(),
                                c.path.clone(),
                            )
                        })
                })
        } else {
            None
        };
        let cache_path = cached.as_ref().map(|c| c.3.clone());
        let (background, device) = if let Some((im, device, issues, _)) = cached {
            cleanup_reviews = issues;
            (im, device)
        } else if neural.as_raw().iter().any(|v| *v > 0) {
            let result = self.inpainting.process(inpainting::Request {
                image: rgb.clone(),
                full_mask: full,
                mask: neural,
                base: base.clone(),
                settings: recipe.clone(),
                directory: directory.into(),
                cancel: cancel.clone(),
                progress: progress.clone(),
                retry: force || incomplete,
            });
            ensure!(!cancel.is_cancelled(), "Cancelled");
            match result {
                Ok(out) => (out.image, out.device),
                Err(e) => {
                    let reason = format!(
                        "Cleanup needs review. {} could not run: {e}. Restore this model/device, then Confirm an edit to retry. Start a fresh page run to use a different cleanup method.",
                        recipe.method.name()
                    );
                    for id in neural_ids {
                        cleanup_reviews.insert(id, reason.clone());
                    }
                    (base, "Cleanup needs review".into())
                }
            }
        } else {
            (
                base,
                if imported.is_some() {
                    "Imported background"
                } else {
                    "Solid fill"
                }
                .into(),
            )
        };
        ensure!(!cancel.is_cancelled(), "Cancelled");
        let output_path =
            crate::safety::page_asset(book, &page.id, &format!("cleanup-{}.png", uid()))?;
        let cache_path = if let Some(path) = cache_path {
            path
        } else {
            crate::assets::save_image(&DynamicImage::ImageRgb8(background.clone()), &output_path)?;
            output_path.to_string_lossy().into()
        };
        page.cleanup = Some(PageCleanup {
            settings: recipe,
            key,
            path: cache_path,
            device: device.clone(),
            reviews: cleanup_reviews.clone(),
        });
        reviews.extend(cleanup_reviews);
        for r in &mut page.regions {
            if crate::safety::has_text(&r.target) || r.prepared {
                r.review = reviews.get(&r.id).cloned();
            }
        }
        checkpoint(page)?;
        ensure!(!cancel.is_cancelled(), "Cancelled");
        (progress)(&device, 0, 0);
        let mut output = background;
        letters.replay(|index, pixmap| {
            ensure!(!cancel.is_cancelled(), "Cancelled");
            let r = &page.regions[index];
            if !reviews.contains_key(&r.id) {
                blend_letters(&mut output, r, pixmap);
            }
            Ok(())
        })?;
        for r in &mut page.regions {
            if crate::safety::has_text(&r.target) || r.prepared {
                r.review = reviews.get(&r.id).cloned();
            }
        }
        Ok(DynamicImage::ImageRgb8(output))
    }
}
fn stamp(mask: &mut GrayImage, c: &cleanup::Cleanup) {
    for (x, y, m) in c.mask.enumerate_pixels() {
        let x = x + c.origin[0];
        let y = y + c.origin[1];
        if m[0] > 0 && x < mask.width() && y < mask.height() {
            mask.put_pixel(x, y, Luma([255]));
        }
    }
}
fn blend_letters(out: &mut RgbImage, r: &Region, p: &tiny_skia::Pixmap) {
    let x = r.bbox[0].max(0.) as u32;
    let y = r.bbox[1].max(0.) as u32;
    for (iy, row) in p.data().chunks(p.width() as usize * 4).enumerate() {
        for (ix, pixel) in row.chunks(4).enumerate() {
            if x + ix as u32 >= out.width() || y + iy as u32 >= out.height() {
                continue;
            }
            let dst = out.get_pixel_mut(x + ix as u32, y + iy as u32);
            let a = pixel[3] as u32;
            for c in 0..3 {
                dst[c] = (pixel[c] as u32 + dst[c] as u32 * (255 - a) / 255).min(255) as u8;
            }
        }
    }
}
use image::GenericImageView;
