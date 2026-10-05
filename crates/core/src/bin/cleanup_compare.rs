//! Re-render saved translations only. No detector, OCR, credentials, or network.
use anyhow::Result;
use image::{GrayImage, Luma};
use serde_json::{Value, json};
use std::{path::PathBuf, time::Instant};
use umanga_core::{cleanup, render::Renderer, store, types::Region};

fn main() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let baseline = root.join("test-output/native");
    let output = root.join("test-output/cleanup-unrestricted");
    std::fs::create_dir_all(&output)?;
    let saved: Vec<Value> = serde_json::from_slice(&std::fs::read(baseline.join("report.json"))?)?;
    let renderer = Renderer::new(&root.join("assets/fonts"))?;
    let mut report = Vec::new();
    for old in saved {
        let name = old["page"].as_str().unwrap();
        let source_path = baseline.join(format!("{name}-original.png"));
        let manga_path = root
            .join("../sample_raw_manga/sample_page_styles")
            .join(format!("{name}.jpg"));
        let source_hash = store::digest(&std::fs::read(&source_path)?);
        let manga_hash = store::digest(&std::fs::read(&manga_path)?);
        let source = image::open(&source_path)?;
        let rgb = source.to_rgb8();
        let mut regions: Vec<Region> = serde_json::from_value(old["regions"].clone())?;
        for r in &mut regions {
            r.review = None;
        }
        let before = Instant::now();
        let rendered = renderer.render_benchmark(&source, &mut regions, "zh-Hans", None)?;
        let elapsed = before.elapsed().as_secs_f64() * 1000.;
        let mut mask = GrayImage::new(source.width(), source.height());
        let mut permitted = mask.clone();
        let mut details = Vec::new();
        let bubbles: Vec<_> = regions.iter().filter_map(|r| r.bubble).collect();
        for r in &regions {
            if r.target.is_empty() {
                continue;
            }
            let result = cleanup::analyze_with_bubbles(&rgb, r, &bubbles);
            let count = result.mask.as_raw().iter().filter(|&&v| v > 0).count();
            if r.review.is_none() {
                for (x, y, p) in result.mask.enumerate_pixels() {
                    if p[0] > 0 {
                        mask.put_pixel(x + result.origin[0], y + result.origin[1], Luma([255]));
                    }
                }
                // Lettering lives in the text rectangle, erasure in the approved glyph mask.
                for y in (r.bbox[1].max(0.) as u32)..(r.bbox[3].ceil() as u32).min(source.height())
                {
                    for x in
                        (r.bbox[0].max(0.) as u32)..(r.bbox[2].ceil() as u32).min(source.width())
                    {
                        permitted.put_pixel(x, y, Luma([255]));
                    }
                }
            }
            details.push(
                        json!({"id":r.id,"maskPixels":count,"fill":result.fill,"maskMode":if result.interior.is_some(){"enclosed"}else{"text-region"},"review":r.review}),
                    );
        }
        let pixels = rendered.to_rgb8();
        let outside = rgb
            .enumerate_pixels()
            .filter(|(x, y, p)| {
                **p != *pixels.get_pixel(*x, *y)
                    && mask.get_pixel(*x, *y)[0] == 0
                    && permitted.get_pixel(*x, *y)[0] == 0
            })
            .count();
        anyhow::ensure!(
            outside == 0,
            "Pixels changed outside cleanup/lettering areas on {name}"
        );
        rendered.save(output.join(format!("{name}.png")))?;
        mask.save(output.join(format!("{name}-mask.png")))?;
        let ready = regions
            .iter()
            .filter(|r| !r.target.is_empty() && r.review.is_none())
            .count();
        println!(
            "{name}: {ready}/{} rendered, {elapsed:.1} ms",
            regions.iter().filter(|r| !r.target.is_empty()).count()
        );
        anyhow::ensure!(
            source_hash == store::digest(&std::fs::read(source_path)?),
            "Baseline source changed"
        );
        anyhow::ensure!(
            manga_hash == store::digest(&std::fs::read(manga_path)?),
            "Original manga changed"
        );
        report.push(json!({"page":name,"renderMs":elapsed,"rendered":ready,"regions":regions,"details":details,"outsideAllowedChanges":outside,"originalSha256":manga_hash,"baselineSourceSha256":source_hash,"originalsUnchanged":true}));
    }
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}
