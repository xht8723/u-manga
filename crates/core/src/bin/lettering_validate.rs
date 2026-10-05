//! Offline lettering regression using saved regions/translations and the production renderer.
use anyhow::{Result, ensure};
use image::{GrayImage, Luma, Rgb, RgbImage};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::Instant};
use tokio_util::sync::CancellationToken;
use umanga_core::{
    cleanup, documents,
    pipeline::Engine,
    render::{region_lettering_text, vertical_lettering},
    store,
    types::*,
};

fn main() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let live_baseline = root.join("test-output/native");
    let baseline = if live_baseline.join("report.json").is_file() {
        live_baseline
    } else {
        root.join("docs/storage-reclamation-2026-09-28/reports/U-Manga/test-output/native")
    };
    let run = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "lettering-format".into());
    ensure!(
        !run.is_empty() && run.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
        "Use a run name containing letters, digits and hyphens"
    );
    let output = root.join("test-output").join(run);
    if std::env::args().any(|a| a == "--verify-formatted") {
        let report: Value = serde_json::from_slice(&std::fs::read(output.join("report.json"))?)?;
        let mut count = 0;
        for page in report["pages"].as_array().unwrap() {
            for region in page["details"].as_array().unwrap() {
                let saved: Region = serde_json::from_value(region["region"].clone())?;
                ensure!(
                    region_lettering_text(&saved) == region["letteringText"].as_str().unwrap(),
                    "Lettering input changed on {} region {}; re-render this corpus",
                    page["page"],
                    region["id"]
                );
                count += 1;
            }
        }
        println!("All {count} saved lettering inputs match the current formatter");
        return Ok(());
    }
    std::fs::create_dir_all(&output)?;
    let book = output.join("validation.umanga");
    ensure!(
        !book.exists(),
        "Validation book already exists; retain it and use a new output folder"
    );
    let saved: Vec<Value> = serde_json::from_slice(&std::fs::read(baseline.join("report.json"))?)?;
    let mut pages = Vec::new();
    for old in &saved {
        let name = old["page"].as_str().unwrap();
        let saved_pixels = baseline.join(format!("{name}-original.png"));
        let source = if saved_pixels.is_file() {
            saved_pixels
        } else {
            root.join("../sample_raw_manga/sample_page_styles")
                .join(format!("{name}.jpg"))
        };
        let mut page = documents::import(&[source.to_string_lossy().into()])?.remove(0);
        page.number = pages.len();
        // Construct ordinary translation fixtures from the frozen detector benchmark.
        // Manual preparation is not part of this corpus; no book is imported or upgraded.
        let mut regions = old["regions"].clone();
        for region in regions.as_array_mut().expect("Frozen region array") {
            region["prepared"] = json!(false);
            // New synthetic format-13 inputs, never migration of a saved book.
            let style = TextStyle::detected(
                region["kind"].as_str().unwrap_or("text"),
                !region["bubble"].is_null(),
            );
            region["style"]["outlineEnabled"] = json!(style.outline_enabled);
            region["style"]["outlineWidthPercent"] = json!(style.outline_width_percent);
            region["style"]["outlineColor"] = json!(style.outline_color);
            // Historical diagnostics are not inputs to a fresh render.
            region["review"] = Value::Null;
        }
        page.regions = serde_json::from_value(regions)?;
        // Only this copied test fixture is changed. Stored books/manual overrides are untouched.
        for region in &mut page.regions {
            region.direction = "auto".into();
            region.review = None;
        }
        pages.push(page);
    }
    store::create(&book, "Lettering validation", &pages)?;
    let model_folder = output.join("no-models-needed");
    let engine = Engine::new(
        model_folder.clone(),
        &root.join("assets/fonts"),
        Arc::new(|_| {}),
        Arc::new(|_| anyhow::bail!("Lettering validation must not call a service")),
    )?;
    let mut settings = TranslationSettings::default();
    settings.cleanup.method = CleanupMethod::Solid;
    let mut report = Vec::new();
    for (old, page) in saved.iter().zip(&mut pages) {
        let name = old["page"].as_str().unwrap();
        let source_path = PathBuf::from(&page.source.path);
        let manga_path = root
            .join("../sample_raw_manga/sample_page_styles")
            .join(format!("{name}.jpg"));
        let source_hash = store::file_hash(&source_path)?;
        let manga_hash = store::file_hash(&manga_path)?;
        let original = documents::load(&page.source)?;
        let rgb = original.to_rgb8();
        let raw: Vec<_> = page
            .regions
            .iter()
            .map(|r| (r.id.clone(), r.source.clone(), r.target.clone()))
            .collect();
        let translation_key = store::cache_key(page, &settings, &ProviderProfile::default(), "")?;
        let start = Instant::now();
        let rendered = engine.render_page(
            &original,
            page,
            &settings,
            &book,
            &model_folder,
            true,
            &CancellationToken::new(),
            Arc::new(|_, _, _| {}),
        )?;
        let elapsed = start.elapsed().as_secs_f64() * 1000.;
        ensure!(
            raw == page
                .regions
                .iter()
                .map(|r| (r.id.clone(), r.source.clone(), r.target.clone()))
                .collect::<Vec<_>>(),
            "Saved text changed"
        );
        ensure!(
            translation_key == store::cache_key(page, &settings, &ProviderProfile::default(), "")?,
            "Translation cache identity changed"
        );
        let mut permitted = GrayImage::new(original.width(), original.height());
        let bubbles: Vec<_> = page.regions.iter().filter_map(|r| r.bubble).collect();
        let mut details = Vec::new();
        for region in &page.regions {
            if region.review.is_none() && !region.target.trim().is_empty() {
                if !region.overlay_only {
                    let mask = cleanup::analyze_with_bubbles(&rgb, region, &bubbles);
                    for (x, y, p) in mask.mask.enumerate_pixels() {
                        if p[0] != 0 {
                            permitted.put_pixel(
                                x + mask.origin[0],
                                y + mask.origin[1],
                                Luma([255]),
                            );
                        }
                    }
                }
                for y in (region.bbox[1].max(0.) as u32)
                    ..(region.bbox[3].ceil() as u32).min(original.height())
                {
                    for x in (region.bbox[0].max(0.) as u32)
                        ..(region.bbox[2].ceil() as u32).min(original.width())
                    {
                        permitted.put_pixel(x, y, Luma([255]));
                    }
                }
            }
            details.push(json!({"id":region.id,"target":region.target,"region":region,"letteringText":region_lettering_text(region),
                "direction":if vertical_lettering(region, "zh-Hans") {"vertical"}else{"horizontal"},"review":region.review}));
        }
        let out = rendered.to_rgb8();
        let outside = rgb
            .enumerate_pixels()
            .filter(|(x, y, p)| {
                **p != *out.get_pixel(*x, *y) && permitted.get_pixel(*x, *y)[0] == 0
            })
            .count();
        ensure!(
            outside == 0,
            "Pixels changed outside cleanup/lettering areas on {name}"
        );
        let image_path = output.join(format!("{name}.png"));
        rendered.save(&image_path)?;
        page.rendered = Some(image_path.to_string_lossy().into());
        let revision = page.revision;
        ensure!(
            store::save_page(&book, page, revision)?,
            "Unexpected validation edit conflict"
        );
        if name == "0006" {
            // Reproduce the provider override on a copy, then verify the four wide labels against explicit horizontal output.
            let mut forced = page.clone();
            for r in forced.regions.iter_mut().take(4) {
                r.direction = "vertical".into();
            }
            let bad = engine.render_page(
                &original,
                &mut forced,
                &settings,
                &book,
                &model_folder,
                false,
                &CancellationToken::new(),
                Arc::new(|_, _, _| {}),
            )?;
            bad.save(output.join("0006-forced-vertical.png"))?;
            let mut sheet = RgbImage::from_pixel(660, 4 * 150, Rgb([228, 228, 228]));
            for (i, region) in page.regions.iter().take(4).enumerate() {
                let auto = engine.renderer.lettering(region, "zh-Hans")?.unwrap();
                let mut horizontal = region.clone();
                horizontal.direction = "horizontal".into();
                ensure!(
                    auto == engine.renderer.lettering(&horizontal, "zh-Hans")?.unwrap(),
                    "Wide label {i} must be horizontal"
                );
                let b = region.bubble.unwrap_or(region.bbox);
                let x = (b[0] - 8.).max(0.) as u32;
                let y = (b[1] - 8.).max(0.) as u32;
                let w = ((b[2] + 8.).ceil() as u32).min(original.width()) - x;
                let h = ((b[3] + 8.).ceil() as u32).min(original.height()) - y;
                for (column, image) in [&original, &bad, &rendered].into_iter().enumerate() {
                    let crop = image.crop_imm(x, y, w, h).thumbnail(216, 144).to_rgb8();
                    image::imageops::replace(
                        &mut sheet,
                        &crop,
                        column as i64 * 220,
                        i as i64 * 150,
                    );
                }
            }
            sheet.save(output.join("0006-wide-labels.png"))?;
        }
        ensure!(
            source_hash == store::file_hash(&source_path)?
                && manga_hash == store::file_hash(&manga_path)?,
            "Originals changed"
        );
        let translated = page
            .regions
            .iter()
            .filter(|r| !r.target.trim().is_empty())
            .count();
        let ready = page
            .regions
            .iter()
            .filter(|r| !r.target.trim().is_empty() && r.review.is_none())
            .count();
        println!("{name}: {ready}/{translated} rendered; {elapsed:.1} ms");
        report.push(json!({"page":name,"renderMs":elapsed,"rendered":ready,"translated":translated,"details":details,
            "outsideAllowedChanges":outside,"originalsUnchanged":true,"originalSha256":manga_hash,
            "baselineSourceSha256":source_hash,"translationCacheUnchanged":true,"rawTextUnchanged":true}));
    }
    let export = output.join("export");
    documents::export(&store::open(&book)?, &export, "images")?;
    for (i, page) in pages.iter().enumerate() {
        ensure!(
            std::fs::read(page.rendered.as_ref().unwrap())?
                == std::fs::read(export.join(format!("{:04}.png", i + 1)))?,
            "Export pixels differ from the Editor render"
        );
    }
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&json!({"pages":report,
        "exportMatchesEditor":true,"ocrRequests":0,"serviceRequests":0,"method":"Production Engine::render_page; solid cleanup; copied Auto regions"}))?,
    )?;
    Ok(())
}
