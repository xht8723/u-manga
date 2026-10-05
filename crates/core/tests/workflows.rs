use std::{path::PathBuf, sync::Arc, time::Duration};
use umanga_core::{documents, pipeline::Engine, store, types::*};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn fixture(folder: &std::path::Path) -> Project {
    let source = folder.join("原稿.png");
    image::RgbImage::from_pixel(300, 300, image::Rgb([255, 255, 255]))
        .save(&source)
        .unwrap();
    let mut pages = documents::import(&[source.to_string_lossy().into()]).unwrap();
    pages[0].fingerprint = store::digest(
        documents::load(&pages[0].source)
            .unwrap()
            .to_rgb8()
            .as_raw(),
    );
    pages[0].regions = vec![Region {
        id: "local".into(),
        bbox: [60., 30., 240., 270.],
        source: "こんにちは".into(),
        overlay_only: true,
        ..Default::default()
    }];
    store::create(&folder.join("本.umanga"), "test", &pages).unwrap()
}
#[tokio::test]
async fn reopened_jobs_deduplicate_and_reuse_cache_without_credentials() {
    let folder = tempfile::tempdir().unwrap();
    let project = fixture(folder.path());
    let profile = ProviderProfile::default();
    let page = &project.pages[0];
    let key = store::cache_key(page, &project.settings, &profile, "").unwrap();
    store::cache_put(
        std::path::Path::new(&project.path),
        &key,
        &[TranslationItem {
            id: "local".into(),
            source: page.regions[0].source.clone(),
            target: "你好".into(),
            direction: "vertical".into(),
        }],
    )
    .unwrap();
    let make = || {
        Engine::new(
            root().join("assets/models"),
            &root().join("assets/fonts"),
            Arc::new(|_| {}),
            Arc::new(|_| anyhow::bail!("Cache hit must never request credentials")),
        )
        .unwrap()
    };
    let engine = make();
    let ids = vec![page.id.clone()];
    let jobs = engine.enqueue(&project.path, &ids, &profile).unwrap();
    assert_eq!(jobs.len(), 1);
    assert!(
        engine
            .enqueue(&project.path, &ids, &profile)
            .unwrap()
            .is_empty()
    );
    engine.control(&jobs[0].id, "pause").unwrap();
    let reopened = make();
    reopened.recover(&project.path).unwrap();
    assert_eq!(reopened.list()[0].status, "paused");
    assert!(
        reopened
            .enqueue(&project.path, &ids, &profile)
            .unwrap()
            .is_empty()
    );
    reopened.control(&jobs[0].id, "resume").unwrap();
    reopened.start();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let job = reopened.list().remove(0);
            if job.status == "complete" {
                break;
            }
            assert_ne!(job.status, "failed", "{:?}", job.error);
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    let saved = store::open(std::path::Path::new(&project.path)).unwrap();
    assert_eq!(saved.pages[0].regions[0].target, "你好");
    assert!(saved.pages[0].rendered.is_some());
}
#[test]
fn exports_preserve_sources_and_pdf_page_dimensions() {
    documents::init_pdfium(&root().join("assets/runtime")).unwrap();
    let folder = tempfile::tempdir().unwrap();
    let mut project = fixture(folder.path());
    let source = PathBuf::from(&project.pages[0].source.path);
    let hash = store::file_hash(&source).unwrap();
    assert!(documents::export(&project, &source, "pdf").is_err());
    assert_eq!(hash, store::file_hash(&source).unwrap());
    project.pages[0].pdf_points = Some([612., 792.]);
    let pdf = folder.path().join("出力.pdf");
    documents::export(&project, &pdf, "pdf").unwrap();
    let imported = documents::import(&[pdf.to_string_lossy().into()]).unwrap();
    assert_eq!(imported[0].pdf_points, Some([612., 792.]));
    let cbz = folder.path().join("出力.cbz");
    documents::export(&project, &cbz, "cbz").unwrap();
    let pages = documents::import(&[cbz.to_string_lossy().into()]).unwrap();
    assert_eq!(
        documents::load(&pages[0].source).unwrap().to_rgb8(),
        documents::load(&project.pages[0].source).unwrap().to_rgb8()
    );
    std::fs::write(&source, b"changed").unwrap();
    assert!(documents::source_health(&project.pages[0]).is_err());
}

#[tokio::test]
async fn cached_provider_direction_cannot_override_auto_or_manual_layout() {
    let folder = tempfile::tempdir_in(root().join("test-output")).unwrap();
    let project = fixture(folder.path());
    let path = std::path::Path::new(&project.path);
    let profile = ProviderProfile::default();
    let mut page = project.pages[0].clone();
    page.regions = vec![
        Region {
            id: "wide-auto".into(),
            bbox: [10., 10., 240., 65.],
            source: "魔界".into(),
            overlay_only: true,
            ..Default::default()
        },
        Region {
            id: "tall-auto".into(),
            bbox: [10., 80., 75., 285.],
            source: "こんにちは".into(),
            overlay_only: true,
            ..Default::default()
        },
        Region {
            id: "manual-horizontal".into(),
            bbox: [100., 80., 165., 285.],
            direction: "horizontal".into(),
            source: "やあ".into(),
            overlay_only: true,
            ..Default::default()
        },
        Region {
            id: "manual-vertical".into(),
            bbox: [190., 80., 285., 285.],
            direction: "vertical".into(),
            source: "やあ".into(),
            overlay_only: true,
            ..Default::default()
        },
    ];
    let revision = page.revision;
    assert!(store::save_page(path, &mut page, revision).unwrap());
    let key = store::cache_key(&page, &project.settings, &profile, "").unwrap();
    let cached = page
        .regions
        .iter()
        .enumerate()
        .map(|(i, r)| TranslationItem {
            id: r.id.clone(),
            source: r.source.clone(),
            target: if i == 0 { "魔界" } else { "你好" }.into(),
            direction: if i == 1 || i == 3 {
                "horizontal"
            } else {
                "vertical"
            }
            .into(),
        })
        .collect::<Vec<_>>();
    store::cache_put(path, &key, &cached).unwrap();
    let engine = Engine::new(
        root().join("assets/models"),
        &root().join("assets/fonts"),
        Arc::new(|_| {}),
        Arc::new(|_| anyhow::bail!("A cached lettering change must not call a service")),
    )
    .unwrap();
    engine
        .enqueue(&project.path, &[page.id.clone()], &profile)
        .unwrap();
    engine.start();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let job = engine.list().remove(0);
            if job.status == "complete" {
                break;
            }
            assert_ne!(job.status, "failed", "{:?}", job.error);
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    let saved = store::page(path, &page.id).unwrap();
    assert_eq!(
        saved
            .regions
            .iter()
            .map(|r| r.direction.as_str())
            .collect::<Vec<_>>(),
        vec!["auto", "auto", "horizontal", "vertical"]
    );
    assert_eq!(saved.regions[0].target, "魔界");
    assert!(saved.regions.iter().all(|r| r.review.is_none()));
    assert_eq!(
        store::cache_key(&saved, &project.settings, &profile, "").unwrap(),
        key
    );
}
