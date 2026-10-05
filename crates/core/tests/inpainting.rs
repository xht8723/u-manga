use image::{DynamicImage, Rgb, RgbImage};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio_util::sync::CancellationToken;
use umanga_core::{documents, pipeline::Engine, store, types::*};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn engine(models: &Path) -> Arc<Engine> {
    Engine::new(
        models.into(),
        &root().join("assets/fonts"),
        Arc::new(|_| {}),
        Arc::new(|_| panic!("Cleanup looked up a credential")),
    )
    .unwrap()
}
fn fixture() -> (tempfile::TempDir, DynamicImage, Page, PathBuf) {
    let dir = tempfile::tempdir_in(root().join("test-output")).unwrap();
    let mut im = RgbImage::from_pixel(120, 120, Rgb([245, 241, 232]));
    for y in 25..80 {
        for x in 35..42 {
            im.put_pixel(x, y, Rgb([10, 10, 10]));
        }
    }
    let source = dir.path().join("原稿.png");
    im.save(&source).unwrap();
    let mut page = documents::import(&[source.to_string_lossy().into()])
        .unwrap()
        .remove(0);
    page.regions = vec![Region {
        bbox: [25., 20., 95., 100.],
        target: "Text".into(),
        direction: "horizontal".into(),
        ..Default::default()
    }];
    let book = dir.path().join("本.umanga");
    store::create(&book, "Test", &[page.clone()]).unwrap();
    (dir, DynamicImage::ImageRgb8(im), page, book)
}
fn render(
    e: &Engine,
    im: &DynamicImage,
    p: &mut Page,
    s: &TranslationSettings,
    b: &Path,
    d: &Path,
    force: bool,
) -> DynamicImage {
    e.render_page(
        im,
        p,
        s,
        b,
        d,
        force,
        &CancellationToken::new(),
        Arc::new(|_, _, _| {}),
    )
    .unwrap()
}
#[test]
fn cleanup_cache_survives_text_and_font_edits_but_not_region_changes() {
    let (d, im, mut p, b) = fixture();
    let e = engine(d.path());
    let mut s = TranslationSettings::default();
    s.cleanup.method = CleanupMethod::Solid;
    render(&e, &im, &mut p, &s, &b, d.path(), true);
    let cache = p.cleanup.clone().unwrap();
    let translation_key = store::cache_key(&p, &s, &ProviderProfile::default(), "").unwrap();
    s.cleanup.method = CleanupMethod::MangaLama;
    p.regions[0].target = "Other".into();
    p.regions[0].style.size = Some(12.);
    render(&e, &im, &mut p, &s, &b, d.path(), false);
    assert_eq!(p.cleanup.as_ref().unwrap().path, cache.path);
    assert_eq!(
        p.cleanup.as_ref().unwrap().settings.method,
        CleanupMethod::Solid
    );
    assert_eq!(
        store::cache_key(&p, &s, &ProviderProfile::default(), "").unwrap(),
        translation_key
    );
    p.regions[0].bbox[2] += 8.;
    render(&e, &im, &mut p, &s, &b, d.path(), false);
    assert_ne!(p.cleanup.as_ref().unwrap().key, cache.key);
    let broken = p.cleanup.as_ref().unwrap().path.clone();
    std::fs::write(&broken, b"corrupt").unwrap();
    render(&e, &im, &mut p, &s, &b, d.path(), false);
    assert_ne!(p.cleanup.as_ref().unwrap().path, broken);
    assert!(image::open(&p.cleanup.as_ref().unwrap().path).is_ok());
    let revision = p.revision;
    assert!(store::save_page(&b, &mut p, revision).unwrap());
    assert_eq!(
        store::page(&b, &p.id).unwrap().cleanup.unwrap().key,
        p.cleanup.unwrap().key
    );
}
#[test]
fn incomplete_neural_cleanup_is_retried_on_confirm_with_its_saved_recipe() {
    let (d, im, mut p, b) = fixture();
    let e = engine(d.path());
    let mut s = TranslationSettings::default();
    s.cleanup.strategy = CleanupStrategy::AllRegions;
    render(&e, &im, &mut p, &s, &b, d.path(), true);
    let failed = p.cleanup.clone().unwrap();
    assert!(!failed.reviews.is_empty());
    assert!(
        !umanga_core::requirements::cached_edit(&p, &p, &e.renderer, &s.target_language),
        "an incomplete cache cannot bypass required models"
    );
    s.cleanup.method = CleanupMethod::Solid;
    render(&e, &im, &mut p, &s, &b, d.path(), false);
    assert_ne!(
        p.cleanup.as_ref().unwrap().path,
        failed.path,
        "explicit Confirm must attempt incomplete cleanup again"
    );
    assert_eq!(p.cleanup.as_ref().unwrap().settings, failed.settings);
    assert_eq!(p.regions[0].target, "Text");
}
#[test]
fn missing_models_preserve_text_and_manual_solid_fill_and_clean_backgrounds_still_work() {
    let (d, im, mut p, b) = fixture();
    let e = engine(d.path());
    let mut s = TranslationSettings::default();
    s.cleanup.strategy = CleanupStrategy::AllRegions;
    render(&e, &im, &mut p, &s, &b, d.path(), true);
    assert_eq!(
        image::open(&p.cleanup.as_ref().unwrap().path)
            .unwrap()
            .to_rgb8(),
        im.to_rgb8()
    );
    assert!(
        p.regions[0]
            .review
            .as_ref()
            .unwrap()
            .to_lowercase()
            .contains("cleanup needs review")
    );
    assert_eq!(p.regions[0].target, "Text");
    p.regions[0].allow_fill = true;

    render(&e, &im, &mut p, &s, &b, d.path(), true);
    let clean = image::open(&p.cleanup.as_ref().unwrap().path)
        .unwrap()
        .to_rgb8();
    assert_ne!(clean, im.to_rgb8());
    assert!(p.regions[0].review.is_none());
    p.regions[0].allow_fill = false;
    p.regions[0].overlay_only = true;
    render(&e, &im, &mut p, &s, &b, d.path(), true);
    assert!(p.regions[0].review.is_none());
    assert_eq!(
        image::open(&p.cleanup.as_ref().unwrap().path)
            .unwrap()
            .to_rgb8(),
        im.to_rgb8()
    );
    p.regions[0].overlay_only = false;
    let bg = d.path().join("clean.png");
    RgbImage::from_pixel(120, 120, Rgb([242, 230, 223]))
        .save(&bg)
        .unwrap();
    p.background = Some(bg.to_string_lossy().into());
    render(&e, &im, &mut p, &s, &b, d.path(), true);
    assert_eq!(p.cleanup.as_ref().unwrap().device, "Imported background");
    assert!(p.regions[0].review.is_none());
}
#[test]
fn missing_translations_and_glyphs_never_erase_originals_and_manual_punctuation_renders() {
    let (d, im, mut p, b) = fixture();
    let e = engine(d.path());
    let mut s = TranslationSettings::default();
    s.cleanup.method = CleanupMethod::Solid;
    for (target, prepared) in [("", false), ("\u{10ffff}", false), ("\u{10ffff}", true)] {
        p.regions[0].prepared = prepared;
        p.regions[0].target = target.into();
        render(&e, &im, &mut p, &s, &b, d.path(), true);
        assert_eq!(
            image::open(&p.cleanup.as_ref().unwrap().path)
                .unwrap()
                .to_rgb8(),
            im.to_rgb8()
        );
        assert!(p.regions[0].review.is_some());
        assert_eq!(p.regions[0].target, target);
    }
    p.regions[0].target = "，。".into();
    p.regions[0].prepared = false;
    let output = render(&e, &im, &mut p, &s, &b, d.path(), true);
    assert!(p.regions[0].review.is_none());
    assert_eq!(p.regions[0].target, "，。");
    assert_ne!(output.to_rgb8(), im.to_rgb8());
}
#[test]
fn explicitly_prepared_blank_regions_are_cleaned_and_later_lettering_reuses_the_background() {
    let (d, im, mut p, b) = fixture();
    let e = engine(d.path());
    let mut s = TranslationSettings::default();
    s.cleanup.method = CleanupMethod::Solid;
    p.regions[0].target.clear();
    p.regions[0].prepared = true;
    render(&e, &im, &mut p, &s, &b, d.path(), true);
    let cache = p.cleanup.clone().unwrap();
    let clean = image::open(&cache.path).unwrap().to_rgb8();
    assert_ne!(clean, im.to_rgb8());
    assert!(p.regions[0].review.is_none());
    p.regions[0].target = "Manual".into();
    p.regions[0].style.size = Some(12.);
    s.cleanup.method = CleanupMethod::MangaLama;
    render(&e, &im, &mut p, &s, &b, d.path(), false);
    assert_eq!(p.cleanup.as_ref().unwrap().key, cache.key);
    assert_eq!(p.cleanup.as_ref().unwrap().path, cache.path);
    p.regions[0].target.clear();
    render(&e, &im, &mut p, &s, &b, d.path(), false);
    assert_eq!(p.cleanup.as_ref().unwrap().key, cache.key);
    let original = image::open(&p.source.path).unwrap().to_rgb8();
    assert_eq!(original, im.to_rgb8());
    let base = p.clone();
    let mut draft = base.clone();
    draft.regions[0].prepared = false;
    assert!(
        umanga_core::editing::merge(&base, &draft, &base)
            .unwrap()
            .regions[0]
            .prepared
    );
}
#[test]
fn manual_cleanup_and_new_imported_background_edits_do_not_require_neural_models() {
    let (d, im, base, _) = fixture();
    let renderer = umanga_core::render::Renderer::new(&root().join("assets/fonts")).unwrap();
    let ready = |p: &Page| umanga_core::requirements::cached_edit(p, &base, &renderer, "zh-Hans");
    assert!(!ready(&base));
    let mut page = base.clone();
    page.regions[0].allow_fill = true;
    assert!(ready(&page));
    page.regions[0].allow_fill = false;
    page.regions[0].overlay_only = true;
    assert!(ready(&page));
    page.regions[0].overlay_only = false;
    let background = d.path().join("imported.png");
    im.save(&background).unwrap();
    page.background = Some(background.to_string_lossy().into());
    assert!(ready(&page));
    RgbImage::new(1, 1).save(&background).unwrap();
    assert!(!ready(&page));
}
#[tokio::test]
async fn cleanup_jobs_cannot_call_ocr_services_or_credentials_and_deduplicate_across_kinds() {
    let (d, _im, p, b) = fixture();
    let e = engine(d.path());
    let mut s = TranslationSettings::default();
    s.cleanup.method = CleanupMethod::Solid;
    s.mode = "local".into();
    e.control_all("stop").unwrap();
    let jobs = e
        .enqueue_captured(
            b.to_str().unwrap(),
            std::slice::from_ref(&p.id),
            &ProviderProfile::default(),
            &s,
            JobKind::Cleanup,
            d.path(),
        )
        .unwrap();
    assert_eq!(jobs[0].kind, JobKind::Cleanup);
    assert_eq!(jobs[0].status, "paused");
    assert!(
        e.enqueue_configured(
            b.to_str().unwrap(),
            std::slice::from_ref(&p.id),
            &ProviderProfile::default(),
            &s
        )
        .unwrap()
        .is_empty()
    );
    s.cleanup.method = CleanupMethod::MangaLama;
    assert_eq!(e.list()[0].settings.cleanup.method, CleanupMethod::Solid);
    let restart = engine(d.path());
    restart.recover(b.to_str().unwrap()).unwrap();
    assert_eq!(restart.list()[0].kind, JobKind::Cleanup);
    assert_eq!(restart.list()[0].status, "paused");
    e.start();
    e.control_all("start").unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if ["complete", "failed"].contains(&e.list()[0].status.as_str()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(e.list()[0].status, "complete", "{:?}", e.list()[0].error);
    let saved = store::page(&b, &p.id).unwrap();
    assert!(saved.rendered.is_some());
    assert_eq!(saved.regions[0].target, "Text");
    assert!(saved.regions[0].source.is_empty());
    // A normal translation submission must retain the saved recipe until explicit re-cleaning.
    s.mode = "vision".into();
    let translated = e
        .enqueue_configured(
            b.to_str().unwrap(),
            std::slice::from_ref(&p.id),
            &ProviderProfile::default(),
            &s,
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let job = e
                .list()
                .into_iter()
                .find(|j| j.id == translated[0].id)
                .unwrap();
            if ["complete", "failed"].contains(&job.status.as_str()) {
                assert_eq!(job.status, "complete", "{:?}", job.error);
                assert_eq!(job.cleanup_model, Some(CleanupMethod::Solid));
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        store::page(&b, &p.id).unwrap().cleanup.unwrap().path,
        saved.cleanup.unwrap().path
    );
}

#[test]
fn repaired_lettering_does_not_reuse_cached_glyph_errors() {
    let (d, im, mut p, b) = fixture();
    let e = engine(d.path());
    let s = TranslationSettings::default();
    for imported in [false, true] {
        p.regions[0].overlay_only = !imported;
        p.background = if imported {
            let path = d.path().join("cleaned.png");
            im.save(&path).unwrap();
            Some(path.to_string_lossy().into())
        } else {
            None
        };
        p.regions[0].target = "\u{10ffff}".into();
        render(&e, &im, &mut p, &s, &b, d.path(), true);
        assert!(p.regions[0].review.is_some());
        let cached = p.cleanup.clone().unwrap();
        assert!(cached.reviews.is_empty());
        p.regions[0].target = "Text".into();
        render(&e, &im, &mut p, &s, &b, d.path(), false);
        assert!(p.regions[0].review.is_none());
        assert_eq!(p.cleanup.as_ref().unwrap().path, cached.path);
    }
}

#[test]
#[ignore = "Uses the locally downloaded benchmark models; run separately from mocked tests"]
fn real_worker_cancels_active_onnx_and_recovers_after_model_switch() {
    use umanga_core::inpainting::{Request, Worker};
    umanga_core::inference::init(&root().join("assets/runtime")).unwrap();
    let worker = Arc::new(Worker::new());
    let make = |method, cancel, progress| {
        let mut mask = image::GrayImage::new(512, 512);
        for y in 220..290 {
            for x in 225..280 {
                mask.put_pixel(x, y, image::Luma([255]));
            }
        }
        Request {
            image: RgbImage::from_pixel(512, 512, Rgb([235, 238, 240])),
            base: RgbImage::from_pixel(512, 512, Rgb([235, 238, 240])),
            full_mask: mask.clone(),
            mask,
            settings: CleanupSettings {
                method,
                strategy: CleanupStrategy::AllRegions,
                device: CleanupDevice::Cpu,
            },
            directory: root().join("test-output/inpainting-integration/models"),
            cancel,
            progress,
            retry: false,
        }
    };
    // Load the slow model first; the cancellation below occurs in a warmed session.
    worker
        .process(make(
            CleanupMethod::MangaLama,
            CancellationToken::new(),
            Arc::new(|_, _, _| {}),
        ))
        .unwrap();
    let cancel = CancellationToken::new();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let request = make(
        CleanupMethod::MangaLama,
        cancel.clone(),
        Arc::new(move |_, n, total| {
            if n == 0 && total > 0 {
                let _ = tx.try_send(());
            }
        }),
    );
    let active = worker.clone();
    let thread = std::thread::spawn(move || active.process(request));
    rx.recv_timeout(Duration::from_secs(10)).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let start = std::time::Instant::now();
    cancel.cancel();
    let error = thread
        .join()
        .unwrap()
        .err()
        .expect("Active inference must be cancelled");
    assert!(error.to_string().contains("Cancelled"));
    assert!(start.elapsed() < Duration::from_secs(10));
    for method in [
        CleanupMethod::Migan,
        CleanupMethod::MangaAot,
        CleanupMethod::MangaLama,
    ] {
        let output = worker
            .process(make(
                method,
                CancellationToken::new(),
                Arc::new(|_, _, _| {}),
            ))
            .unwrap();
        assert_eq!(output.device, "CPU");
        assert_eq!(output.image.get_pixel(0, 0), &Rgb([235, 238, 240]));
    }
}

#[tokio::test]
#[ignore = "Uses a downloaded model to stop an actual ONNX job"]
async fn real_stop_all_waits_for_inference_exit_then_resumes_one_attempt() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use umanga_core::inpainting::Request;
    umanga_core::inference::init(&root().join("assets/runtime")).unwrap();
    let (d, _im, p, b) = fixture();
    let directory = root().join("test-output/inpainting-integration/models");
    let entered = Arc::new(AtomicBool::new(false));
    let notification = entered.clone();
    let e = Engine::new(
        directory.clone(),
        &root().join("assets/fonts"),
        Arc::new(move |j| {
            if j.stage == "cleaning" && j.stage_detail.contains("tile 0/") {
                notification.store(true, Ordering::SeqCst);
            }
        }),
        Arc::new(|_| panic!("Cleanup cannot request credentials")),
    )
    .unwrap();
    let s = TranslationSettings {
        cleanup: CleanupSettings {
            method: CleanupMethod::MangaLama,
            strategy: CleanupStrategy::AllRegions,
            device: CleanupDevice::Cpu,
        },
        ..Default::default()
    };
    let warm = Request {
        image: RgbImage::from_pixel(512, 512, Rgb([245, 245, 245])),
        base: RgbImage::from_pixel(512, 512, Rgb([245, 245, 245])),
        full_mask: image::GrayImage::from_pixel(512, 512, image::Luma([255])),
        mask: image::GrayImage::from_pixel(512, 512, image::Luma([255])),
        settings: s.cleanup.clone(),
        directory: directory.clone(),
        cancel: CancellationToken::new(),
        progress: Arc::new(|_, _, _| {}),
        retry: false,
    };
    e.inpainting.process(warm).unwrap();
    let id = e
        .enqueue_cleanup(b.to_str().unwrap(), std::slice::from_ref(&p.id), &s)
        .unwrap()[0]
        .id
        .clone();
    e.start();
    tokio::time::timeout(Duration::from_secs(15), async {
        while !entered.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    e.control_all("stop").unwrap();
    assert!(e.is_held());
    assert_eq!(e.list()[0].status, "paused");
    tokio::time::timeout(Duration::from_secs(10), async {
        while e.snapshot(None)[0].stopping {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(store::page(&b, &p.id).unwrap().rendered.is_none());
    e.control_all("start").unwrap();
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let j = e.list().remove(0);
            if ["complete", "failed"].contains(&j.status.as_str()) {
                assert_eq!(j.status, "complete", "{:?}", j.error);
                assert_eq!(j.id, id);
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(e.list().len(), 1);
    assert_eq!(
        store::page(&b, &p.id)
            .unwrap()
            .cleanup
            .unwrap()
            .settings
            .method,
        CleanupMethod::MangaLama
    );
    drop(d);
}
