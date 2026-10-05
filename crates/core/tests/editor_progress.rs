use serde_json::json;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use umanga_core::{documents, editing, pipeline::Engine, store, types::*};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn fixture(dir: &Path, count: usize) -> Project {
    let image = dir.join("原稿.png");
    image::RgbImage::from_pixel(300, 300, image::Rgb([255, 255, 255]))
        .save(&image)
        .unwrap();
    let mut pages = documents::import(&[image.to_string_lossy().into()]).unwrap();
    pages[0].fingerprint = store::digest(
        documents::load(&pages[0].source)
            .unwrap()
            .to_rgb8()
            .as_raw(),
    );
    pages[0].regions = (0..count)
        .map(|i| Region {
            id: format!("r{i}"),
            bbox: [20., 20., 200., 200.],
            source: "こんにちは".into(),
            overlay_only: true,
            ..Default::default()
        })
        .collect();
    store::create(&dir.join("book.umanga"), "Editor fixture", &pages).unwrap()
}
#[test]
fn restored_region_keeps_its_place_without_reordering_new_job_regions() {
    let dir = tempfile::tempdir_in(root().join("test-output")).unwrap();
    let book = fixture(dir.path(), 2);
    let mut base = book.pages[0].clone();
    let restored = base.regions.remove(0);
    let mut draft = base.clone();
    draft.regions.insert(0, restored.clone());
    let mut latest = base.clone();
    latest.regions.insert(
        0,
        Region {
            id: "new".into(),
            ..Default::default()
        },
    );
    let merged = editing::merge(&base, &draft, &latest).unwrap();
    assert_eq!(
        merged
            .regions
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        vec!["new", "r0", "r1"]
    );
    assert_eq!(merged.regions[1].source, restored.source);
    assert!(editing::merge(&base, &draft, &merged).is_err());
}
fn engine(notify: Arc<dyn Fn(Job) + Send + Sync>) -> Arc<Engine> {
    Engine::new(
        root().join("assets/models"),
        &root().join("assets/fonts"),
        notify,
        Arc::new(|_| Ok("mock".into())),
    )
    .unwrap()
}
async fn terminal(engine: &Arc<Engine>) -> Job {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let j = engine.list().remove(0);
            if ["failed", "complete"].contains(&j.status.as_str()) {
                // The in-memory status precedes the ordered durable write.
                let durable = store::jobs(Path::new(&j.project))
                    .unwrap()
                    .into_iter()
                    .find(|saved| saved.id == j.id)
                    .unwrap();
                if durable.status == j.status {
                    return durable;
                }
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn partial_batches_survive_provider_failure_and_restart_with_precise_step() {
    let dir = tempfile::tempdir_in(root().join("test-output")).unwrap();
    let book = fixture(dir.path(), 49);
    let server = MockServer::builder().start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(|r:&wiremock::Request|{
        if String::from_utf8_lossy(&r.body).contains("r48") { return ResponseTemplate::new(400).set_body_json(json!({"error":{"message":"Injected batch two failure","type":"invalid_request_error"}})) }
        let items=(0..48).map(|i|json!({"id":format!("r{i}"),"source":"こんにちは","target":"你好"})).collect::<Vec<_>>();
        ResponseTemplate::new(200).set_body_json(json!({"id":"mock","model":"test","choices":[{"index":0,"message":{"role":"assistant","content":json!({"regions":items}).to_string()},"finish_reason":"stop"}]}))
    }).mount(&server).await;
    let profile = ProviderProfile {
        endpoint: format!("{}/v1", server.uri()),
        simple_translation: false,
        model: "test".into(),
        rate_limit: 20.,
        ..Default::default()
    };
    let settings = TranslationSettings {
        mode: "local".into(),
        ..Default::default()
    };
    let events = Arc::new(Mutex::new(vec![]));
    let observed = events.clone();
    let worker = engine(Arc::new(move |j| {
        observed.lock().unwrap().push(j);
    }));
    worker
        .enqueue_configured(&book.path, &[book.pages[0].id.clone()], &profile, &settings)
        .unwrap();
    worker.start();
    let job = terminal(&worker).await;
    assert_eq!(job.status, "failed");
    assert_eq!(job.stage, "translating");
    let saved = store::page(Path::new(&book.path), &book.pages[0].id).unwrap();
    assert_eq!(
        saved
            .regions
            .iter()
            .filter(|r| !r.target.is_empty())
            .count(),
        48
    );
    assert!(saved.rendered.is_none());
    assert_eq!(job.page_revision, Some(saved.revision));
    assert!(
        job.steps
            .iter()
            .any(|s| s.stage == "ocr" && s.status == "skipped")
    );
    assert!(
        job.steps
            .iter()
            .any(|s| s.stage == "translating" && s.status == "failed" && s.error.is_some())
    );
    assert!(
        events
            .lock()
            .unwrap()
            .iter()
            .any(|j| j.status == "running" && j.page_revision.is_some())
    );
    let reopened = engine(Arc::new(|_| {}));
    reopened.recover(&book.path).unwrap();
    assert_eq!(
        reopened.list()[0].steps.last().unwrap().error,
        job.steps.last().unwrap().error
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}
#[tokio::test]
async fn translation_is_saved_before_cleanup_failure() {
    let dir = tempfile::tempdir_in(root().join("test-output")).unwrap();
    let book = fixture(dir.path(), 1);
    let path = Path::new(&book.path);
    let mut p = book.pages[0].clone();
    p.background = Some(dir.path().join("missing.png").to_string_lossy().into());
    store::save_page(path, &mut p, 0).unwrap();
    let profile = ProviderProfile {
        simple_translation: false,
        endpoint: "http://127.0.0.1:1/v1".into(),
        ..Default::default()
    };
    let key = store::cache_key(&p, &book.settings, &profile, "").unwrap();
    store::cache_put(
        path,
        &key,
        &[TranslationItem {
            id: "r0".into(),
            source: "こんにちは".into(),
            target: "阿·布=朋友！你好，世界。".into(),
            direction: String::new(),
        }],
    )
    .unwrap();
    let worker = engine(Arc::new(|_| {}));
    worker
        .enqueue(&book.path, &[p.id.clone()], &profile)
        .unwrap();
    worker.start();
    let job = terminal(&worker).await;
    assert_eq!(job.status, "failed");
    assert_eq!(job.stage, "cleaning");
    let saved = store::page(path, &p.id).unwrap();
    assert_eq!(saved.regions[0].target, "阿·布=朋友！\n你好\n世界");
    assert_eq!(
        store::cache_get(path, &key).unwrap().unwrap()[0].target,
        "阿·布=朋友！你好，世界。"
    );
    let mut edited = saved.clone();
    edited.regions[0].target = "甲，乙。·=！\n手工\n\n换行".into();
    store::save_page(path, &mut edited, saved.revision).unwrap();
    let reopened = store::page(path, &p.id).unwrap();
    assert_eq!(reopened.regions[0].target, edited.regions[0].target);
    assert!(
        job.steps
            .iter()
            .any(|s| s.stage == "translating" && s.status == "complete")
    );
    assert!(
        job.steps
            .iter()
            .any(|s| s.stage == "cleaning" && s.error.is_some())
    );
}
#[test]
fn confirmed_edits_merge_fields_and_do_not_resurrect_deleted_regions() {
    let dir = tempfile::tempdir_in(root().join("test-output")).unwrap();
    let book = fixture(dir.path(), 1);
    let base = &book.pages[0];
    let mut draft = base.clone();
    draft.regions[0].style.size = Some(22.);
    draft.regions[0].target = "manual".into();
    let mut latest = base.clone();
    latest.revision = 4;
    latest.regions[0].source = "new OCR".into();
    latest.regions[0].style.color = "#ff0000".into();
    let merged = editing::merge(base, &draft, &latest).unwrap();
    assert_eq!(merged.revision, 4);
    assert_eq!(merged.regions[0].source, "new OCR");
    assert_eq!(merged.regions[0].target, "manual");
    assert_eq!(merged.regions[0].style.color, "#ff0000");
    assert_eq!(merged.regions[0].style.size, Some(22.));
    latest.regions.clear();
    assert!(editing::merge(base, &draft, &latest).is_err());
}
#[tokio::test]
async fn edit_reservation_holds_the_book_without_changing_queued_jobs() {
    let dir = tempfile::tempdir_in(root().join("test-output")).unwrap();
    let initial = fixture(dir.path(), 1);
    let mut pages = initial.pages.clone();
    let mut second = pages[0].clone();
    second.id = uid();
    second.number = 1;
    pages.push(second);
    let book = store::create(&dir.path().join("two-pages.umanga"), "Two pages", &pages).unwrap();
    let worker = engine(Arc::new(|_| {}));
    let ids = [book.pages[0].id.clone()];
    let profile = ProviderProfile::default();
    let jobs = worker.enqueue(&book.path, &ids, &profile).unwrap();
    worker
        .enqueue(&book.path, &[pages[1].id.clone()], &profile)
        .unwrap();
    let guard = worker.pause_for_edit(&book.path, &ids[0]).await.unwrap();
    assert_eq!(
        worker
            .list()
            .iter()
            .find(|j| j.page_id == ids[0])
            .unwrap()
            .status,
        "queued"
    );
    assert_eq!(
        worker
            .list()
            .iter()
            .find(|j| j.page_id == pages[1].id)
            .unwrap()
            .status,
        "queued"
    );
    assert!(worker.enqueue(&book.path, &ids, &profile).is_err());
    assert!(worker.quiesce(&book.path).await.is_err());
    worker.control(&jobs[0].id, "cancel").unwrap();
    drop(guard);
    assert_eq!(worker.enqueue(&book.path, &ids, &profile).unwrap().len(), 1);
}
#[test]
fn geometry_edits_clear_stale_balloon_bounds_but_preserve_text_and_identity() {
    let dir = tempfile::tempdir_in(root().join("test-output")).unwrap();
    let book = fixture(dir.path(), 1);
    let mut base = book.pages[0].clone();
    base.regions[0].bbox = [10., 10., 30., 40.];
    base.regions[0].bubble = Some([5., 5., 60., 60.]);
    let mut draft = base.clone();
    draft.regions[0].bbox = [15., 15., 35., 45.];
    let mut latest = base.clone();
    latest.regions[0].source = "latest OCR".into();
    let inside = editing::merge(&base, &draft, &latest).unwrap();
    assert_eq!(inside.regions[0].bubble, base.regions[0].bubble);
    draft.regions[0].bbox = [70., 70., 100., 110.];
    let moved = editing::merge(&base, &draft, &latest).unwrap();
    assert_eq!(moved.regions[0].bubble, None);
    assert_eq!(moved.regions[0].id, base.regions[0].id);
    assert_eq!(moved.regions[0].source, "latest OCR");
    assert_eq!(moved.regions[0].target, base.regions[0].target);
    assert_eq!(moved.regions[0].style.font, base.regions[0].style.font);
    assert!(
        !serde_json::to_value(&moved.regions[0])
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("strokes")
    );
}
