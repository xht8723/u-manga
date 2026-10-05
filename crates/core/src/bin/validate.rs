use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::Instant,
};
use umanga_core::{documents, inference, inference::Inference, render::Renderer, store, types::*};
fn read(p: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&std::fs::read(p)?)?)
}
fn main() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let repo = root.parent().unwrap();
    let output = root.join("test-output/native");
    std::fs::create_dir_all(&output)?;
    inference::init(&root.join("assets/runtime"))?;
    documents::init_pdfium(&root.join("assets/runtime"))?;
    let mut infer = Inference::new(root.join("assets/models"));
    let renderer = Renderer::new(&root.join("assets/fonts"))?;
    let mut rows = Vec::new();
    let mut sources = Vec::new();
    let args = std::env::args().collect::<Vec<_>>();
    let run_ocr = args.iter().any(|v| v == "--ocr");
    let limit = args
        .iter()
        .find_map(|v| {
            v.strip_prefix("--limit=")
                .and_then(|v| v.parse::<usize>().ok())
        })
        .unwrap_or(19);
    let mut paths = std::fs::read_dir(repo.join("sample_raw_manga/sample_page_styles"))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("jpg"))
        .collect::<Vec<_>>();
    paths.sort();
    for path in paths.into_iter().take(limit) {
        let stem = path.file_stem().unwrap().to_str().unwrap();
        let im = image::open(&path)?;
        let start = Instant::now();
        let detected = infer.detect(&im)?;
        let detect_ms = start.elapsed().as_millis();
        let old = read(&repo.join(format!(
            "benchmark_results/detection/rtdetr_int8_cpu/{stem}.json"
        )))?;
        let cloud = read(&repo.join(format!(
            "benchmark_results/cloud/crops_rtdetr_int8_cpu/{stem}.json"
        )))?;
        let mut regions = Vec::new();
        for (i, b) in old["boxes"].as_array().context("boxes")?.iter().enumerate() {
            let bbox = serde_json::from_value::<[f32; 4]>(b["box"].clone())?;
            let t = cloud["parsed"]["regions"]
                .as_array()
                .and_then(|rows| rows.iter().find(|r| r["id"].as_u64() == Some(i as u64)));
            regions.push(Region {
                id: i.to_string(),
                bbox,
                bubble: serde_json::from_value(b["bubble"].clone()).unwrap_or(None),
                kind: b["kind"].as_str().unwrap_or("text").into(),
                // Let the renderer apply the current unrounded proportion rule.
                direction: "auto".into(),
                source: t.and_then(|t| t["jp"].as_str()).unwrap_or("").into(),
                target: t.and_then(|t| t["zh"].as_str()).unwrap_or("").into(),
                ..Default::default()
            });
        }
        let mut ocr = vec![];
        let mut ocr_ms = 0;
        let mut pp = String::new();
        if run_ocr {
            let crops = regions
                .iter()
                .map(|r| inference::crop(&im, r.bbox))
                .collect::<Vec<_>>();
            let t = Instant::now();
            ocr = infer.manga(&crops)?;
            ocr_ms = t.elapsed().as_millis();
            if stem == "0005" {
                pp = infer.pp(&crops[0], "ja")?
            }
        } else if args.iter().any(|v| v == "--reuse-ocr") {
            let previous = read(&output.join(format!("{stem}.json")))?;
            ocr = serde_json::from_value(previous["ocr"].clone())?;
            ocr_ms = previous["ocrMs"].as_u64().unwrap_or(0) as u128;
            pp = previous["ppSample"].as_str().unwrap_or("").into();
        }
        let start = Instant::now();
        let rendered = renderer.render_benchmark(&im, &mut regions, "zh-Hans", None)?;
        let render_ms = start.elapsed().as_millis();
        rendered.save(output.join(format!("{stem}.png")))?;
        let row = json!({"page":stem,"detected":detected.len(),"referenceRegions":regions.len(),"detectMs":detect_ms,"ocrMs":ocr_ms,"renderMs":render_ms,"ocr":ocr,"ppSample":pp,"regions":regions});
        store::atomic_write(
            &output.join(format!("{stem}.json")),
            &serde_json::to_vec_pretty(&row)?,
        )?;
        println!(
            "{stem}: detect {detect_ms}ms, OCR {ocr_ms}ms, render {render_ms}ms, review {}/{}",
            regions.iter().filter(|r| r.review.is_some()).count(),
            regions.len()
        );
        rows.push(row);
        sources.push(path.to_string_lossy().into_owned());
    }
    store::atomic_write(
        &output.join("report.json"),
        &serde_json::to_vec_pretty(&rows)?,
    )?;
    let path = output.join(format!("validation-{}.umanga", uid()));
    let pages = documents::import(&sources)?;
    let mut project = store::create(&path, "19-page native validation", &pages)?;
    for p in &mut project.pages {
        let stem = Path::new(&p.name).file_stem().unwrap().to_string_lossy();
        let data = read(&output.join(format!("{stem}.json")))?;
        p.regions = serde_json::from_value(data["regions"].clone())?;
        p.rendered = Some(output.join(format!("{stem}.png")).to_string_lossy().into());
        p.status = "complete".into();
        store::save_page(&path, p, 0)?;
    }
    documents::export(&project, &output.join("translated.cbz"), "cbz")?;
    documents::export(&project, &output.join("translated.pdf"), "pdf")?;
    let pdf = documents::import(&[output.join("translated.pdf").to_string_lossy().into()])?;
    assert_eq!(pdf.len(), project.pages.len());
    println!("Book: {}", path.display());
    Ok(())
}
