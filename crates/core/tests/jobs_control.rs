use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use umanga_core::{documents, pipeline::Engine, store, types::*};
fn fixture(count: usize) -> (tempfile::TempDir, Project) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let t = tempfile::tempdir_in(root.join("test-output")).unwrap();
    let source = t.path().join("原稿.png");
    image::RgbImage::from_pixel(20, 30, image::Rgb([255, 255, 255]))
        .save(&source)
        .unwrap();
    let base = documents::import(&[source.to_string_lossy().into()])
        .unwrap()
        .remove(0);
    let pages: Vec<_> = (0..count)
        .map(|i| {
            let mut p = base.clone();
            p.id = uid();
            p.number = i;
            p
        })
        .collect();
    let project = store::create(&t.path().join("本.umanga"), "本", &pages).unwrap();
    (t, project)
}
fn engine() -> Arc<Engine> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    Engine::new(
        root.join("assets/models"),
        &root.join("assets/fonts"),
        Arc::new(|_| {}),
        Arc::new(|_| anyhow::bail!("No live requests")),
    )
    .unwrap()
}
#[test]
fn stale_retry_validation_cannot_undo_cancel() {
    let (_t, p) = fixture(1);
    let e = engine();
    let jobs = e
        .enqueue(
            &p.path,
            &[p.pages[0].id.clone()],
            &ProviderProfile::default(),
        )
        .unwrap();
    let id = &jobs[0].id;
    let retry = e.request_job_control(id).unwrap();
    e.control(id, "cancel").unwrap();
    assert!(e.control_requested(id, "retry", retry).is_err());
    e.block_job_requested(id, "Late validation error", Some(retry));
    assert_eq!(e.list()[0].status, "cancelled");
    assert!(e.control(id, "retry").is_err());
}
#[test]
fn library_ownership_includes_books_with_only_region_work() {
    let (_t, p) = fixture(1);
    let e = engine();
    let operation = uid();
    let guard = e
        .reserve_region(&operation, &p.path, &p.pages[0].id)
        .unwrap();
    assert!(e.list().is_empty());
    assert!(e.operation_books().contains(&p.path));
    e.cancel_region(&operation);
    assert!(e.operation_books().contains(&p.path));
    drop(guard);
    assert!(!e.operation_books().contains(&p.path));
}
#[test]
fn start_all_releases_hold_and_keeps_missing_requirements_paused() {
    let (_t, p) = fixture(3);
    let e = engine();
    e.control_all("stop").unwrap();
    let jobs = e
        .enqueue(
            &p.path,
            &p.pages.iter().map(|p| p.id.clone()).collect::<Vec<_>>(),
            &ProviderProfile::default(),
        )
        .unwrap();
    e.control(&jobs[2].id, "cancel").unwrap();
    let blocked =
        std::collections::HashMap::from([(jobs[0].id.clone(), "Download Manga AOT".into())]);
    let request = e.request_control("start").unwrap();
    let result = e.control_all_ready("start", request, &blocked).unwrap();
    assert!(!e.is_held());
    assert_eq!(result.errors.len(), 1);
    let list = e.list();
    assert_eq!(
        list.iter().find(|j| j.id == jobs[0].id).unwrap().status,
        "paused"
    );
    assert_eq!(
        list.iter().find(|j| j.id == jobs[1].id).unwrap().status,
        "queued"
    );
    assert_eq!(
        list.iter().find(|j| j.id == jobs[2].id).unwrap().status,
        "cancelled"
    );
}
#[test]
fn hold_new_jobs_resume_retry_exclusions_and_restart() {
    let (_t, p) = fixture(6);
    let e = engine();
    let provider = ProviderProfile::default();
    let jobs = e
        .enqueue(
            &p.path,
            &p.pages[..4]
                .iter()
                .map(|p| p.id.clone())
                .collect::<Vec<_>>(),
            &provider,
        )
        .unwrap();
    e.control(&jobs[3].id, "cancel").unwrap();
    e.control_all("stop").unwrap();
    assert!(e.is_held());
    let added = e
        .enqueue(&p.path, &[p.pages[4].id.clone()], &provider)
        .unwrap();
    assert_eq!(added[0].status, "paused");
    assert!(
        e.control(&added[0].id, "resume")
            .unwrap_err()
            .to_string()
            .contains("Start all")
    );
    let reopened = engine();
    reopened.recover(&p.path).unwrap();
    assert!(!reopened.is_held());
    assert!(
        reopened
            .list()
            .iter()
            .filter(|j| j.status != "cancelled")
            .all(|j| j.status == "paused")
    );
    assert_eq!(
        reopened
            .enqueue(&p.path, &[p.pages[5].id.clone()], &provider)
            .unwrap()[0]
            .status,
        "queued"
    );
    let mut saved = store::jobs(Path::new(&p.path)).unwrap();
    for j in &mut saved {
        if j.id == jobs[0].id {
            j.status = "failed".into();
        }
        if j.id == jobs[1].id {
            j.status = "complete".into();
        }
    }
    store::save_jobs(Path::new(&p.path), &saved).unwrap();
    let retry = engine();
    retry.recover(&p.path).unwrap();
    let result = retry.control_all("start").unwrap();
    assert!(result.errors.is_empty());
    let all = retry.list();
    assert_eq!(
        all.iter().find(|j| j.id == jobs[0].id).unwrap().status,
        "queued"
    );
    assert_eq!(
        all.iter().find(|j| j.id == jobs[1].id).unwrap().status,
        "complete"
    );
    assert_eq!(
        all.iter().find(|j| j.id == jobs[3].id).unwrap().status,
        "cancelled"
    );
    assert_eq!(all.iter().filter(|j| j.status == "queued").count(), 4);
}
#[test]
fn superseded_failures_are_not_retried_and_storage_failures_are_explicit() {
    let (_t, p) = fixture(2);
    let e = engine();
    let provider = ProviderProfile::default();
    let mut jobs = e
        .enqueue(
            &p.path,
            &p.pages.iter().map(|p| p.id.clone()).collect::<Vec<_>>(),
            &provider,
        )
        .unwrap();
    jobs[0].status = "failed".into();
    jobs[1].status = "failed".into();
    let mut newer = jobs[0].clone();
    newer.id = uid();
    newer.status = "complete".into();
    newer.created = "9999".into();
    store::save_jobs(
        Path::new(&p.path),
        &[jobs[0].clone(), jobs[1].clone(), newer],
    )
    .unwrap();
    let recovered = engine();
    recovered.recover(&p.path).unwrap();
    assert_eq!(recovered.control_all("start").unwrap().changed, 1);
    assert_eq!(
        recovered
            .list()
            .iter()
            .find(|j| j.id == jobs[0].id)
            .unwrap()
            .status,
        "failed"
    );
    std::fs::remove_file(&p.path).unwrap();
    let result = recovered.control_all("stop").unwrap();
    assert_eq!(result.errors.len(), 1);
    assert!(recovered.is_held());
    assert_eq!(
        recovered
            .list()
            .iter()
            .find(|j| j.id == jobs[1].id)
            .unwrap()
            .status,
        "paused"
    );
}
#[test]
fn blocked_storage_does_not_block_state_reads_or_stop_gate() {
    let (_t, p) = fixture(1000);
    let e = engine();
    e.recover(&p.path).unwrap();
    let db = store::connection(Path::new(&p.path)).unwrap();
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    let worker = e.clone();
    let path = p.path.clone();
    let ids = p.pages.iter().map(|p| p.id.clone()).collect::<Vec<_>>();
    let thread = std::thread::spawn(move || {
        worker
            .enqueue(&path, &ids, &ProviderProfile::default())
            .unwrap()
    });
    std::thread::sleep(Duration::from_millis(80));
    let timer = Instant::now();
    e.hold();
    let _ = e.snapshot(None);
    assert!(timer.elapsed() < Duration::from_millis(200));
    db.execute_batch("ROLLBACK").unwrap();
    let added = thread.join().unwrap();
    assert_eq!(added.len(), 1000);
    assert!(added.iter().all(|j| j.status == "paused"));
    assert_eq!(store::jobs(Path::new(&p.path)).unwrap().len(), 1000);
}
#[test]
fn latest_bulk_request_wins_while_waiting_for_recovery() {
    let (_t, p) = fixture(1);
    let e = engine();
    e.enqueue(
        &p.path,
        &[p.pages[0].id.clone()],
        &ProviderProfile::default(),
    )
    .unwrap();
    let start = e.request_control("start").unwrap();
    let stop = e.request_control("stop").unwrap();
    e.control_all_requested("stop", stop).unwrap();
    e.control_all_requested("start", start).unwrap();
    assert!(e.is_held());
    assert_eq!(e.list()[0].status, "paused");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_during_mock_request_prevents_late_completion_and_double_dispatch() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
    let server = MockServer::builder().start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({}))
                .set_delay(Duration::from_secs(3)),
        )
        .mount(&server)
        .await;
    let (_t, p) = fixture(1);
    let path = Path::new(&p.path);
    let mut page = p.pages[0].clone();
    page.regions = vec![Region {
        source: "こんにちは".into(),
        bbox: [2., 2., 18., 28.],
        overlay_only: true,
        ..Default::default()
    }];
    let revision = page.revision;
    store::save_page(path, &mut page, revision).unwrap();
    let settings = TranslationSettings {
        mode: "local".into(),
        ..Default::default()
    };
    store::settings(path, &settings).unwrap();
    let provider = ProviderProfile {
        simple_translation: false,
        endpoint: server.uri(),
        model: "mock".into(),
        vision: false,
        ..Default::default()
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let e = Engine::new(
        root.join("assets/models"),
        &root.join("assets/fonts"),
        Arc::new(|_| {}),
        Arc::new(|_| Ok("mock-key".into())),
    )
    .unwrap();
    e.enqueue(&p.path, &[page.id.clone()], &provider).unwrap();
    e.start();
    tokio::time::timeout(Duration::from_secs(30), async {
        while server.received_requests().await.unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    e.control_all("stop").unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while e.snapshot(None).iter().any(|j| j.stopping) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(e.list()[0].status, "paused");
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    assert!(store::page(path, &page.id).unwrap().rendered.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn context_books_keep_page_order_while_other_books_share_bounded_workers() {
    use parking_lot::Mutex;
    use std::collections::{HashMap, HashSet};
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
    let server = MockServer::builder().start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({}))
                .set_delay(Duration::from_millis(180)),
        )
        .mount(&server)
        .await;
    let (_a, a) = fixture(3);
    let (_b, b) = fixture(1);
    for p in [&a, &b] {
        for (index, original) in p.pages.iter().enumerate() {
            let mut page = original.clone();
            page.regions = vec![Region {
                source: format!("source-{index}"),
                bbox: [2., 2., 18., 28.],
                overlay_only: true,
                ..Default::default()
            }];
            let revision = page.revision;
            store::save_page(Path::new(&p.path), &mut page, revision).unwrap();
        }
    }
    let log = Arc::new(Mutex::new(Vec::<Job>::new()));
    let notifications = log.clone();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let engine = Engine::new(
        root.join("assets/models"),
        &root.join("assets/fonts"),
        Arc::new(move |j| notifications.lock().push(j)),
        Arc::new(|_| Ok("mock-key".into())),
    )
    .unwrap();
    let provider = ProviderProfile {
        simple_translation: false,
        endpoint: server.uri(),
        model: "mock".into(),
        rate_limit: 20.,
        ..Default::default()
    };
    let settings = TranslationSettings {
        mode: "local".into(),
        context_pages: 1,
        ..Default::default()
    };
    let reversed: Vec<_> = a.pages.iter().rev().map(|p| p.id.clone()).collect();
    engine
        .enqueue_configured(&a.path, &reversed, &provider, &settings)
        .unwrap();
    engine.prioritize(&a.path, &[a.pages[2].id.clone()]);
    engine
        .enqueue_configured(&b.path, &[b.pages[0].id.clone()], &provider, &settings)
        .unwrap();
    engine.start();
    // Cold font/runtime initialization can contend with other integration tests.
    // Order, ownership, worker count and exact requests are asserted below, not by wall time.
    tokio::time::timeout(Duration::from_secs(30), async {
        while engine
            .list()
            .iter()
            .any(|j| ["queued", "running"].contains(&j.status.as_str()))
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let mut active = HashMap::new();
    let mut seen = HashSet::new();
    let mut a_order = vec![];
    let mut peak = 0;
    for job in &*log.lock() {
        if job.stage == "starting" && job.status == "running" && seen.insert(job.id.clone()) {
            assert!(
                !active.values().any(|path| path == &job.project),
                "Context book ran overlapping pages"
            );
            active.insert(job.id.clone(), job.project.clone());
            peak = peak.max(active.len());
            if job.project == a.path {
                a_order.push(job.page_id.clone());
            }
        }
        if ["complete", "failed", "cancelled"].contains(&job.status.as_str()) {
            active.remove(&job.id);
        }
    }
    assert_eq!(
        a_order,
        a.pages.iter().map(|p| p.id.clone()).collect::<Vec<_>>()
    );
    assert_eq!(
        peak, 2,
        "Independent books should share the two-worker limit"
    );
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 4);
    assert!(
        requests
            .iter()
            .any(|r| String::from_utf8_lossy(&r.body).contains("source-1")
                && String::from_utf8_lossy(&r.body).contains("source-0")),
        "Later page request must include preceding cached text"
    );
}
