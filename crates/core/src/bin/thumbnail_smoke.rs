//! Local thumbnail size/reuse check. Never runs OCR or providers.
use anyhow::{Context, Result};
use std::{fs, io::Cursor, path::PathBuf, time::Instant};
use umanga_core::{documents, thumbnails::Cache};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let source = args.next().context("Expected source folder")?;
    let output = PathBuf::from(args.next().context("Expected output folder")?);
    let pages = documents::import(&[source])?;
    let mut cache = Cache::new(output.join("cache"))?;
    let start = Instant::now();
    let mut png_baseline_bytes = 0;
    let mut previews = Vec::new();
    for page in &pages {
        let mut png = Cursor::new(Vec::new());
        documents::load(&page.source)?
            .thumbnail(320, 480)
            .write_to(&mut png, image::ImageFormat::Png)?;
        png_baseline_bytes += png.into_inner().len();
        previews.push(cache.get(&page.source)?);
    }
    let first_ms = start.elapsed().as_millis();
    let start = Instant::now();
    for (page, first) in pages.iter().zip(&previews) {
        anyhow::ensure!(
            *first == cache.get(&page.source)?,
            "Cache hit changed preview"
        );
    }
    let hit_ms = start.elapsed().as_millis();
    let files: Vec<_> = fs::read_dir(output.join("cache"))?.collect::<std::io::Result<_>>()?;
    let cache_bytes: u64 = files
        .iter()
        .map(|e| e.metadata().map(|m| m.len()))
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .sum();
    anyhow::ensure!(
        files.len() == pages.len(),
        "Unexpected duplicate cache files"
    );
    let result = serde_json::json!({"pages":pages.len(),"pngBaselineBytes":png_baseline_bytes,"sharedJpegBytes":cache_bytes,"filesAfterTwoPasses":files.len(),"firstPassIncludingPngComparisonMs":first_ms,"cacheHitPassMs":hit_ms});
    fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
