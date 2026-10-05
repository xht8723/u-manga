use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio_util::sync::CancellationToken;
use umanga_core::{
    documents,
    library::{self, Metadata},
    pipeline::Engine,
    queue, store,
    types::*,
};

fn picture(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    image::RgbImage::from_pixel(24, 32, image::Rgb([250, 250, 250]))
        .save(path)
        .unwrap();
}
fn engine() -> Arc<Engine> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    Engine::new(
        root.join("assets/models"),
        &root.join("assets/fonts"),
        Arc::new(|_| {}),
        Arc::new(|_| anyhow::bail!("No provider calls in queue tests")),
    )
    .unwrap()
}
fn job(path: &Path, page: &Page) -> Job {
    Job {
        kind: JobKind::Translation,
        cleanup_model: None,
        steps: vec![],
        page_revision: None,
        fresh: false,
        glossary_checkpoint: None,
        model_directory: String::new(),
        id: uid(),
        project: path.to_string_lossy().into(),
        page_id: page.id.clone(),
        status: "running".into(),
        stage: "translating".into(),
        stage_detail: "Batch 2 / 4 · 6 regions".into(),
        error: None,
        created: now(),
        elapsed_ms: 1200,
        device: String::new(),
        settings: TranslationSettings::default(),
        provider: ProviderProfile::default(),
    }
}

#[test]
fn settings_changed_during_readiness_do_not_change_the_validated_job() {
    let t = tempfile::tempdir().unwrap();
    let original = t.path().join("page.png");
    picture(&original);
    let hash = store::file_hash(&original).unwrap();
    let pages = documents::import(&[original.to_string_lossy().into()]).unwrap();
    let path = t.path().join("book.umanga");
    store::create(&path, "Book", &pages).unwrap();
    let profile = ProviderProfile::default();
    let validated = TranslationSettings {
        mode: "local".into(),
        provider_id: profile.id.clone(),
        ..Default::default()
    };
    let changed = TranslationSettings {
        mode: "vision".into(),
        target_language: "en".into(),
        ..validated.clone()
    };
    store::settings(&path, &changed).unwrap();
    let jobs = engine()
        .enqueue_configured(
            path.to_str().unwrap(),
            &[pages[0].id.clone()],
            &profile,
            &validated,
        )
        .unwrap();
    assert_eq!(jobs[0].settings.mode, "local");
    assert_eq!(jobs[0].settings.target_language, "zh-Hans");
    assert_eq!(store::jobs(&path).unwrap()[0].settings.mode, "local");
    assert_eq!(store::open(&path).unwrap().settings.target_language, "en");
    assert_eq!(store::file_hash(&original).unwrap(), hash);
}

#[test]
fn queue_locations_follow_renames_and_page_moves_without_reading_originals() {
    let t = tempfile::tempdir().unwrap();
    let raw = t.path().join("原稿");
    for name in ["Chapter 1/1.png", "Chapter 1/2.png", "Chapter 2/1.png"] {
        picture(&raw.join(name));
    }
    let preview = library::scan(
        &[raw.to_string_lossy().into()],
        true,
        &CancellationToken::new(),
    )
    .unwrap();
    let root = t.path().join("library");
    let mut book = library::create(
        &root,
        Metadata {
            title: "本".into(),
            ..Default::default()
        },
        preview,
        None,
    )
    .unwrap();
    let path = PathBuf::from(&book.path);
    let page = store::page(&path, &book.chapters[1].page_ids[0]).unwrap();
    let task = job(&path, &page);
    let location = queue::describe(vec![task.clone()])[0]
        .location
        .clone()
        .unwrap();
    assert_eq!((location.page_number, location.chapter_pages), (1, 1));
    assert_eq!(location.chapter_title, "Chapter 2");
    let id = book.chapters[1].page_ids.remove(0);
    book.chapters[0].page_ids.insert(1, id);
    book.chapters[0].title = "Volume 二".into();
    book = library::organize(&root, &path, book.revision, book.chapters, vec![], vec![]).unwrap();
    book.metadata.title = "Renamed book".into();
    library::update(&root, &path, book.metadata, book.overrides, book.revision).unwrap();
    fs::remove_file(&page.source.path).unwrap();
    let described = queue::describe(vec![task.clone(), task]);
    let location = described[0].location.as_ref().unwrap();
    assert_eq!(location.book_title, "Renamed book");
    assert_eq!(location.chapter_title, "Volume 二");
    assert_eq!((location.page_number, location.chapter_pages), (2, 3));
    assert_eq!(described[0].job.page_id, page.id);
    let mut missing = described[0].job.clone();
    missing.page_id = "removed".into();
    let mut cache = queue::LocationCache::default();
    assert!(
        cache.describe_views(vec![umanga_core::job_view::RuntimeView {
            job: missing,
            sequence: 0,
            stopping: false,
            timer_running: false
        }])[0]
            .location
            .is_none()
    );
}

#[test]
fn recovery_and_controls_keep_the_last_processing_step_until_resumed() {
    let t = tempfile::tempdir().unwrap();
    let original = t.path().join("page.png");
    picture(&original);
    let pages = documents::import(&[original.to_string_lossy().into()]).unwrap();
    let path = t.path().join("book.umanga");
    store::create(&path, "Book", &pages).unwrap();
    let task = job(&path, &pages[0]);
    store::save_job(&path, &task).unwrap();
    let e = engine();
    e.recover(path.to_str().unwrap()).unwrap();
    let recovered = e.list().remove(0);
    assert_eq!(recovered.status, "paused");
    assert_eq!(recovered.stage, "translating");
    assert_eq!(recovered.elapsed_ms, 1200);
    e.control(&task.id, "resume").unwrap();
    assert_eq!(e.list()[0].stage, "waiting");
    assert!(e.list()[0].stage_detail.is_empty());
    e.control(&task.id, "cancel").unwrap();
    assert_eq!(e.list()[0].status, "cancelled");
    assert_eq!(store::jobs(&path).unwrap()[0].status, "cancelled");
    e.enqueue(
        path.to_str().unwrap(),
        &[pages[0].id.clone()],
        &ProviderProfile::default(),
    )
    .unwrap();
    assert!(
        e.control(&task.id, "resume")
            .unwrap_err()
            .to_string()
            .contains("cancelled jobs cannot be changed")
    );
    assert_eq!(
        e.list().iter().filter(|job| job.status == "queued").count(),
        1
    );
}

#[tokio::test]
async fn failed_tasks_report_the_actual_step_without_any_provider_calls() {
    let t = tempfile::tempdir().unwrap();
    let original = t.path().join("page.png");
    picture(&original);
    let pages = documents::import(&[original.to_string_lossy().into()]).unwrap();
    let path = t.path().join("book.umanga");
    store::create(&path, "Book", &pages).unwrap();
    fs::remove_file(original).unwrap();
    let e = engine();
    e.enqueue(
        path.to_str().unwrap(),
        &[pages[0].id.clone()],
        &ProviderProfile::default(),
    )
    .unwrap();
    e.start();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if e.list()[0].status == "failed" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let task = e.list().remove(0);
    assert_eq!(task.stage, "loading");
    assert!(task.error.is_some());
    assert_eq!(store::jobs(&path).unwrap()[0].stage, "loading");
}

#[tokio::test]
async fn credential_failures_keep_the_translation_step_and_batch_detail() {
    let t = tempfile::tempdir().unwrap();
    let original = t.path().join("page.png");
    picture(&original);
    let mut pages = documents::import(&[original.to_string_lossy().into()]).unwrap();
    pages[0].fingerprint = store::digest(
        documents::load(&pages[0].source)
            .unwrap()
            .to_rgb8()
            .as_raw(),
    );
    pages[0].regions = vec![Region {
        bbox: [4., 4., 20., 28.],
        source: "こんにちは".into(),
        ..Default::default()
    }];
    let path = t.path().join("book.umanga");
    store::create(&path, "Book", &pages).unwrap();
    let e = engine();
    e.enqueue(
        path.to_str().unwrap(),
        &[pages[0].id.clone()],
        &ProviderProfile::default(),
    )
    .unwrap();
    e.start();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if e.list()[0].status == "failed" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let task = e.list().remove(0);
    assert_eq!(task.stage, "translating");
    assert_eq!(task.stage_detail, "Batch 1 / 1 · 1 region");
    assert!(task.error.unwrap().contains("No provider calls"));
    assert_eq!(store::jobs(&path).unwrap()[0].stage, "translating");
}

#[test]
fn unsupported_book_and_library_formats_are_rejected_without_upgrading_them() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("old.umanga");
    let c = rusqlite::Connection::open(&path).unwrap();
    c.execute_batch("CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);INSERT INTO meta VALUES('version','2');").unwrap();
    drop(c);
    let hash = store::file_hash(&path).unwrap();
    assert!(
        store::open(&path)
            .unwrap_err()
            .to_string()
            .contains("Unsupported book format")
    );
    assert!(library::import_book(&t.path().join("new"), &path).is_err());
    assert_eq!(store::file_hash(&path).unwrap(), hash);
    let root = t.path().join("old-library");
    fs::create_dir(&root).unwrap();
    let index = root.join("library.sqlite");
    let c = rusqlite::Connection::open(&index).unwrap();
    c.execute_batch("CREATE TABLE books(id TEXT PRIMARY KEY,data TEXT NOT NULL)")
        .unwrap();
    drop(c);
    let hash = store::file_hash(&index).unwrap();
    assert!(
        library::list(&root)
            .unwrap_err()
            .to_string()
            .contains("Unsupported library format")
    );
    assert_eq!(store::file_hash(&index).unwrap(), hash);
}
