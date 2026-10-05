//! Offline detector observations only; no production ordering or book writes.
use anyhow::{Result, ensure};
use serde::Deserialize;
use serde_json::json;
use std::{path::PathBuf, time::Instant};
use umanga_core::{inference, inference::Inference};

#[derive(Deserialize)]
struct Input {
    page: String,
    source: PathBuf,
    panels: Vec<[u32; 4]>,
}
fn main() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let args = std::env::args().collect::<Vec<_>>();
    ensure!(
        args.len() == 3,
        "Pass an input manifest and a new output file"
    );
    let input: Vec<Input> = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    ensure!(input.len() == 19, "Use the retained 19-page corpus");
    ensure!(
        !std::path::Path::new(&args[2]).exists(),
        "Retain previous observations; use a new output file"
    );
    inference::init(&root.join("assets/runtime"))?;
    let mut detector = Inference::new(root.join("assets/models"));
    let mut rows = Vec::new();
    for page in input {
        let original = std::fs::read(&page.source)?;
        let sha = umanga_core::store::digest(&original);
        let image = image::load_from_memory(&original)?;
        let start = Instant::now();
        let mut baseline = detector.detect(&image)?;
        let baseline_ms = start.elapsed().as_secs_f64() * 1000.;
        for (i, region) in baseline.iter_mut().enumerate() {
            region.id = format!("b{}", i + 1);
        }
        let start = Instant::now();
        let mut cropped = Vec::new();
        let mut crop_timings = Vec::new();
        for (index, [left, top, right, bottom]) in page.panels.iter().copied().enumerate() {
            ensure!(
                right > left && bottom > top && right <= image.width() && bottom <= image.height(),
                "Invalid panel crop"
            );
            let crop = image.crop_imm(left, top, right - left, bottom - top);
            let started = Instant::now();
            let mut regions = detector.detect(&crop)?;
            crop_timings.push(json!({"panel":index,"ms":started.elapsed().as_secs_f64()*1000.,"count":regions.len()}));
            for (local, region) in regions.iter_mut().enumerate() {
                for rect in std::iter::once(&mut region.bbox).chain(region.bubble.iter_mut()) {
                    rect[0] += left as f32;
                    rect[2] += left as f32;
                    rect[1] += top as f32;
                    rect[3] += top as f32;
                }
                region.id = format!("p{}-{}", index + 1, local + 1);
            }
            cropped.extend(regions);
        }
        let crop_ms = start.elapsed().as_secs_f64() * 1000.;
        ensure!(
            sha == umanga_core::store::digest(&std::fs::read(&page.source)?),
            "Original changed during observation"
        );
        println!(
            "{}: baseline={} ({baseline_ms:.0}ms), panel crops={} ({crop_ms:.0}ms), panels={}",
            page.page,
            baseline.len(),
            cropped.len(),
            page.panels.len()
        );
        rows.push(json!({"page":page.page,"source":page.source,"sha256":sha,"width":image.width(),"height":image.height(),"panels":page.panels,"baseline":baseline,"cropped":cropped,"baselineMs":baseline_ms,"cropMs":crop_ms,"cropTimings":crop_timings}));
        std::fs::write(&args[2], serde_json::to_vec_pretty(&rows)?)?;
    }
    Ok(())
}
