//! Isolated native Editor/Jobs fixture; no models, external servers or user books.
use anyhow::Result;
use std::path::{Path, PathBuf};
use umanga_core::{documents, library, store, types::*};
fn main() -> Result<()> {
    let app = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = app
        .join("test-output/editor-progress")
        .join(format!("native-{}", uid()));
    std::fs::create_dir_all(&root)?;
    let source = root.join("original.png");
    let mut image = image::RgbImage::from_pixel(480, 640, image::Rgb([255, 255, 255]));
    for y in 80..160 {
        for x in 90..110 {
            image.put_pixel(x, y, image::Rgb([0, 0, 0]));
        }
    }
    image.save(&source)?;
    let mut pages = documents::import(&[source.to_string_lossy().into()])?;
    pages[0].regions = vec![Region {
        id: "dialogue".into(),
        bbox: [60., 40., 230., 240.],
        bubble: Some([30., 20., 260., 260.]),
        source: "こんにちは".into(),
        target: "你好".into(),
        ..Default::default()
    }];
    let library = root.join("library");
    let book = library::create(
        &library,
        library::Metadata {
            title: "Editor progress fixture".into(),
            ..Default::default()
        },
        library::ImportPreview {
            omitted_page_ids: vec![],
            chapters: vec![library::Chapter {
                id: uid(),
                title: "Chapter 1".into(),
                page_ids: pages.iter().map(|p| p.id.clone()).collect(),
                read: false,
            }],
            pages: pages.clone(),
            warnings: vec![],
        },
        None,
    )?;
    let mut settings = AppSettings::default();
    settings.setup.completed = true;
    settings.setup.step = "review".into();
    settings.library_directory = library.to_string_lossy().into();
    settings.translation.cleanup.method = CleanupMethod::Solid;
    settings.translation.mode = "local".into();
    let mut job = Job {
        id: uid(),
        project: book.path.clone(),
        page_id: pages[0].id.clone(),
        status: "failed".into(),
        stage: "translating".into(),
        stage_detail: "1 / 2 translations saved".into(),
        error: Some("Synthetic offline service failure".into()),
        created: now(),
        elapsed_ms: 400,
        device: "CPU".into(),
        settings: settings.translation.clone(),
        provider: ProviderProfile {
            endpoint: "http://127.0.0.1:1/v1".into(),
            ..Default::default()
        },
        kind: JobKind::Translation,
        cleanup_model: None,
        model_directory: settings.model_directory().to_string_lossy().into(),
        steps: vec![],
        page_revision: Some(0),
        fresh: false,
        glossary_checkpoint: None,
    };
    for stage in ["loading", "detecting", "ocr"] {
        job.record_step(stage, "complete", "Saved", None);
    }
    job.record_step(
        "translating",
        "failed",
        "Partial translations saved",
        job.error.clone(),
    );
    store::save_jobs(Path::new(&book.path), &[job])?;
    store::atomic_write(
        &root.join("preferences.json"),
        &serde_json::to_vec_pretty(&settings)?,
    )?;
    let info =
        serde_json::json!({"root":root,"book":book.path,"pageId":pages[0].id,"original":source});
    store::atomic_write(
        &app.join("test-output/editor-progress/native-fixture.json"),
        &serde_json::to_vec_pretty(&info)?,
    )?;
    println!("{info}");
    Ok(())
}
