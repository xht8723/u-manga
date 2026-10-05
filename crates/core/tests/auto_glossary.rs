use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use umanga_core::{documents, glossary, library, pipeline::Engine, store, types::*};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn fixture(dir: &Path, pages: usize, regions: usize, vision: bool) -> Project {
    let original = dir.join("原稿.png");
    image::RgbImage::from_pixel(240, 320, image::Rgb([255, 255, 255]))
        .save(&original)
        .unwrap();
    let base = documents::import(&[original.to_string_lossy().into()])
        .unwrap()
        .remove(0);
    let pages = (0..pages)
        .map(|n| {
            let mut p = base.clone();
            p.id = uid();
            p.number = n;
            p.regions = (0..regions)
                .map(|_| Region {
                    id: uid(),
                    bbox: [20., 20., 200., 290.],
                    source: if vision {
                        String::new()
                    } else {
                        "アリスは城へ".into()
                    },
                    overlay_only: true,
                    ..Default::default()
                })
                .collect();
            p
        })
        .collect::<Vec<_>>();
    let project = store::create(&dir.join("book.umanga"), "Glossary fixture", &pages).unwrap();
    // These tests explicitly exercise opt-in learning, not fresh-install defaults.
    let book = library::open(Path::new(&project.path)).unwrap();
    let mut glossary = book.glossary;
    glossary.enabled = true;
    glossary.auto_detect = true;
    library::save_glossary(dir, Path::new(&project.path), glossary.revision, glossary).unwrap();
    project
}
fn engine() -> Arc<Engine> {
    Engine::new(
        root().join("assets/models"),
        &root().join("assets/fonts"),
        Arc::new(|_| {}),
        Arc::new(|_| Ok("mock".into())),
    )
    .unwrap()
}
async fn done(e: &Arc<Engine>) -> Vec<Job> {
    tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            let list = e.list();
            if !list.is_empty()
                && list
                    .iter()
                    .all(|j| ["complete", "failed", "cancelled"].contains(&j.status.as_str()))
                && e.snapshot(None).iter().all(|j| !j.timer_running)
            {
                return list;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap()
}
fn response(value: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({"id":"mock","model":"test","choices":[{"index":0,"message":{"role":"assistant","content":value.to_string()},"finish_reason":"stop"}]}))
}
fn text_response(text: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(
        json!({"choices":[{"message":{"role":"assistant","content":text},"finish_reason":"stop"}]}),
    )
}
async fn server(book: &Project, bad_glossary: bool, bad_translation: bool) -> MockServer {
    let server = MockServer::builder().start().await;
    let ids = book
        .pages
        .iter()
        .flat_map(|p| p.regions.iter().map(|r| r.id.clone()))
        .collect::<Vec<_>>();
    Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(move |r: &wiremock::Request| {
        let body: Value = serde_json::from_slice(&r.body).unwrap();
        let system = body["messages"][0]["content"].as_str().unwrap();
        let user = body["messages"][1]["content"].to_string();
        if system.starts_with("识别并提取") || system.starts_with("GLOSSARY-CUSTOM") {
            assert!(!user.contains("image_url"));
            assert!(!user.contains("Existing terms:") && !user.contains("previous dialogue"));
            assert!(user.contains("Source:") && !user.contains("Translation:") && !user.contains("爱丽丝去了城堡"));
            assert!(!user.contains("Region ID:"));
            assert!(body.get("response_format").is_none());
            if bad_glossary { return text_response("malformed"); }
            return text_response("[1] アリス:爱丽丝\n[2] 架空:Invented\nbroken row");
        }
        if system.starts_with("Transcribe") {
            let current = ids.iter().filter(|id| user.contains(id.as_str())).collect::<Vec<_>>();
            assert!(!current.is_empty());
            return response(json!({"regions":current.iter().map(|id| json!({"id":id,"source":"アリスは城へ","target":""})).collect::<Vec<_>>()}));
        }
        if bad_translation { return ResponseTemplate::new(400).set_body_json(json!({"error":{"message":"Injected translation failure","type":"invalid_request_error"}})); }
        if system.starts_with("Translate current text ") {
            let input = body["messages"][1]["content"].as_str().unwrap();
            assert_eq!(body["messages"].as_array().unwrap().len(), 2);
            assert!(!input.contains("Region ID:") && !input.contains("image_url") && !input.contains(system));
            let current = input.rsplit("Current texts:\n").next().unwrap();
            let count = current.lines().filter(|s| s.starts_with('[')).count();
            assert!(count > 0);
            assert!(body.get("response_format").is_none());
            return text_response(&(1..=count).map(|i| format!("[{i}] 爱丽丝去了城堡")).collect::<Vec<_>>().join("\n"));
        }
        let current = ids.iter().filter(|id| user.contains(id.as_str())).collect::<Vec<_>>();
        assert!(!current.is_empty());
        response(json!({"regions":current.iter().map(|id| json!({"id":id,"source":"アリスは城へ","target":"爱丽丝去了城堡"})).collect::<Vec<_>>()}))
    }).mount(&server).await;
    server
}

fn provider(server: &MockServer) -> ProviderProfile {
    ProviderProfile {
        simple_translation: false,
        endpoint: format!("{}/v1", server.uri()),
        model: "test".into(),
        rate_limit: 20.,
        vision: true,
        ..Default::default()
    }
}
fn settings(vision: bool) -> TranslationSettings {
    TranslationSettings {
        mode: if vision { "vision" } else { "local" }.into(),
        auto_glossary: true,
        glossary_enabled: true,
        ..Default::default()
    }
}

#[tokio::test]
async fn restart_reuses_sources_and_successful_extraction_batches_before_translation() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let dir = tempfile::tempdir_in(root().join("test-output")).unwrap();
    let book = fixture(dir.path(), 1, 7, true);
    let server = MockServer::start().await;
    let worker = engine();
    let stop = worker.clone();
    let requests = Arc::new(AtomicUsize::new(0));
    let count = requests.clone();
    let ids: Vec<_> = book.pages[0].regions.iter().map(|r| r.id.clone()).collect();
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(move |r: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&r.body).unwrap();
            let system = body["messages"][0]["content"].as_str().unwrap();
            let user = body["messages"][1]["content"].to_string();
            if system.starts_with("识别并提取") {
                assert!(
                    user.contains("アリスは城へ")
                        && !user.contains("Translation:")
                        && !user.contains("image_url")
                );
                if count.fetch_add(1, Ordering::SeqCst) == 1 {
                    stop.hold();
                }
                text_response("[1] アリス:爱丽丝")
            } else {
                assert!(
                    system.starts_with("Transcribe"),
                    "No translation before extraction completes"
                );
                response(
                    json!({"regions":ids.iter().filter(|id| user.contains(id.as_str())).map(|id|
                json!({"id":id,"source":"アリスは城へ","target":""})).collect::<Vec<_>>()}),
                )
            }
        })
        .mount(&server)
        .await;
    worker
        .enqueue_configured(
            &book.path,
            &[book.pages[0].id.clone()],
            &provider(&server),
            &settings(true),
        )
        .unwrap();
    worker.start();
    tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            if worker.list().iter().all(|j| j.status == "paused")
                && worker.snapshot(None).iter().all(|j| !j.timer_running)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let first = store::jobs(Path::new(&book.path)).unwrap().remove(0);
    let checkpoint = first.glossary_checkpoint.as_ref().unwrap();
    assert_eq!(checkpoint.processed.len(), 6);
    assert!(!checkpoint.completed);
    assert_eq!(checkpoint.added, 1);
    assert!(
        serde_json::to_value(checkpoint)
            .unwrap()
            .get("translations")
            .is_none()
    );
    let page = store::page(Path::new(&book.path), &book.pages[0].id).unwrap();
    assert!(
        page.regions
            .iter()
            .all(|r| r.source == "アリスは城へ" && r.target.is_empty())
    );
    server.reset().await;
    let ids: Vec<_> = page.regions.iter().map(|r| r.id.clone()).collect();
    Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(move |r: &wiremock::Request| {
        let body: Value = serde_json::from_slice(&r.body).unwrap();
        let system = body["messages"][0]["content"].as_str().unwrap();
        let user = body["messages"][1]["content"].to_string();
        assert!(!system.starts_with("Transcribe"), "Saved sources survive restart");
        if system.starts_with("识别并提取") { assert!(!user.contains("image_url")); text_response("NONE") }
        else {
            assert!(user.contains("爱丽丝"), "Current translation receives previously learned terms");
            response(json!({"regions":ids.iter().filter(|id| user.contains(id.as_str())).map(|id|
                json!({"id":id,"source":"アリスは城へ","target":"爱丽丝！去了城堡。"})).collect::<Vec<_>>()}))
        }
    }).mount(&server).await;
    let resumed = engine();
    resumed.recover(&book.path).unwrap();
    resumed.control(&first.id, "retry").unwrap();
    resumed.start();
    let saved = done(&resumed).await.remove(0);
    assert_eq!(saved.status, "complete", "{:?}", saved.error);
    assert_eq!(server.received_requests().await.unwrap().len(), 3);
    assert_eq!(saved.glossary_checkpoint.unwrap().added, 1);
    assert_eq!(
        library::open(Path::new(&book.path))
            .unwrap()
            .glossary
            .entries
            .len(),
        1
    );
    assert!(
        store::page(Path::new(&book.path), &book.pages[0].id)
            .unwrap()
            .regions
            .iter()
            .all(|r| r.target == umanga_core::render::format_translation(r, "爱丽丝！去了城堡。"))
    );
}

#[tokio::test]
async fn extraction_precedes_cache_lookup_and_manual_targets_are_ineligible() {
    let dir = tempfile::tempdir_in(root().join("test-output")).unwrap();
    let book = fixture(dir.path(), 1, 2, false);
    let book_path = Path::new(&book.path);
    let mut page = store::page(book_path, &book.pages[0].id).unwrap();
    page.regions[1].target = "Manual text".into();
    page.regions[1].source = "SAVED-MANUAL-SOURCE".into();
    store::save_page(book_path, &mut page, 0).unwrap();
    let server = server(&book, true, false).await;
    let profile = provider(&server);
    let worker = engine();
    worker
        .enqueue_configured(&book.path, &[page.id.clone()], &profile, &settings(false))
        .unwrap();
    worker.start();
    assert_eq!(done(&worker).await[0].status, "complete");
    let mut page = store::page(book_path, &page.id).unwrap();
    page.regions[0].target.clear();
    let revision = page.revision;
    store::save_page(book_path, &mut page, revision).unwrap();
    server.reset().await;
    let id = page.regions[0].id.clone();
    Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(move |r: &wiremock::Request| {
        let body: Value = serde_json::from_slice(&r.body).unwrap();
        let system = body["messages"][0]["content"].as_str().unwrap();
        let user = body["messages"][1]["content"].to_string();
        assert!(!user.contains("SAVED-MANUAL-SOURCE") && !user.contains("Manual text"));
        if system.starts_with("识别并提取") { text_response("[1] アリス:NEW-PREFERRED-NAME") }
        else {
            assert!(user.contains("NEW-PREFERRED-NAME"), "New terms change the translation request/cache key");
            response(json!({"regions":[{"id":id,"source":"アリスは城へ","target":"NEW-PREFERRED-NAME went to the castle"}]}))
        }
    }).mount(&server).await;
    let resumed = engine();
    resumed
        .enqueue_configured(&book.path, &[page.id.clone()], &profile, &settings(false))
        .unwrap();
    resumed.start();
    assert_eq!(done(&resumed).await[0].status, "complete");
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        2,
        "Old translation cache cannot bypass newly learned terms"
    );
    let saved = store::page(book_path, &page.id).unwrap();
    assert!(saved.regions[0].target.contains("NEW-PREFERRED-NAME"));
    assert_eq!(saved.regions[1].target, "Manual text");
    // A subsequent application reuses source-only extraction and the new translation cache.
    let mut saved = saved;
    saved.regions[0].target.clear();
    let revision = saved.revision;
    store::save_page(book_path, &mut saved, revision).unwrap();
    server.reset().await;
    let cached = engine();
    let submitted = cached
        .enqueue_configured(&book.path, &[saved.id], &profile, &settings(false))
        .unwrap()
        .remove(0)
        .id;
    cached.start();
    let job = done(&cached)
        .await
        .into_iter()
        .find(|j| j.id == submitted)
        .unwrap();
    assert_eq!(job.status, "complete", "{:?}", job.error);
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn simple_toggle_keeps_vision_reading_and_extraction_before_translation() {
    let dir = tempfile::tempdir_in(root().join("test-output")).unwrap();
    let book = fixture(dir.path(), 1, 7, true);
    let server = server(&book, false, false).await;
    let e = engine();
    let mut p = provider(&server);
    p.simple_translation = true;
    e.enqueue_configured(&book.path, &[book.pages[0].id.clone()], &p, &settings(true))
        .unwrap();
    e.start();
    let job = done(&e).await.remove(0);
    assert_eq!(job.status, "complete", "{:?}", job.error);
    let requests = server.received_requests().await.unwrap();
    let bodies = requests
        .iter()
        .map(|r| serde_json::from_slice::<Value>(&r.body).unwrap())
        .collect::<Vec<_>>();
    let simple = bodies
        .iter()
        .filter(|b| {
            b["messages"][0]["content"]
                .as_str()
                .unwrap()
                .starts_with("Translate current text ")
        })
        .count();
    assert_eq!(simple, 0);
    assert_eq!(bodies.len(), 6);
    assert!(
        bodies
            .iter()
            .all(|b| b["messages"].as_array().unwrap().len() == 2)
    );
    let page = store::page(Path::new(&book.path), &book.pages[0].id).unwrap();
    assert!(
        page.regions
            .iter()
            .all(|r| r.source == "アリスは城へ" && r.target == "爱丽丝去了城堡")
    );
}
#[tokio::test]
async fn vision_glossary_path_uses_captured_prompts_after_source_reading() {
    let dir = tempfile::tempdir_in(root().join("test-output")).unwrap();
    let book = fixture(dir.path(), 1, 1, true);
    let server = server(&book, false, false).await;
    let engine = engine();
    let mut profile = provider(&server);
    profile.instructions = umanga_core::instructions::Overrides {
        text_translation: Some("TEXT-CUSTOM".into()),
        vision_translation: Some("VISION-CUSTOM".into()),
        glossary_detection: Some("GLOSSARY-CUSTOM".into()),
        ..Default::default()
    };
    engine
        .enqueue_configured(
            &book.path,
            &[book.pages[0].id.clone()],
            &profile,
            &settings(true),
        )
        .unwrap();
    profile.instructions.text_translation = Some("LATER-EDIT".into());
    engine.start();
    let job = done(&engine).await.remove(0);
    assert_eq!(job.status, "complete", "{:?}", job.error);
    let requests = server.received_requests().await.unwrap();
    let systems = requests
        .iter()
        .map(|r| {
            serde_json::from_slice::<Value>(&r.body).unwrap()["messages"][0]["content"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(systems.len(), 3);
    assert!(systems[0].starts_with("Transcribe"));
    assert!(systems[1].starts_with("GLOSSARY-CUSTOM"));
    assert!(systems[2].starts_with("VISION-CUSTOM"));
    assert!(!systems.join("\n").contains("TEXT-CUSTOM"));
    assert!(!systems.join("\n").contains("LATER-EDIT"));
}
#[tokio::test]
async fn disabling_either_glossary_switch_makes_no_extraction_requests() {
    for (enabled, automatic) in [(false, false), (false, true), (true, false)] {
        let dir = tempfile::tempdir_in(root().join("test-output")).unwrap();
        let book = fixture(dir.path(), 1, 1, false);
        let current = library::open(Path::new(&book.path)).unwrap();
        let mut glossary = current.glossary;
        glossary.enabled = enabled;
        glossary.auto_detect = automatic;
        glossary.entries.push(GlossaryEntry {
            source: "アリス".into(),
            target: "爱丽丝".into(),
        });
        library::save_glossary(
            dir.path(),
            Path::new(&book.path),
            glossary.revision,
            glossary.clone(),
        )
        .unwrap();
        let server = server(&book, false, false).await;
        let e = engine();
        let mut settings = settings(false);
        settings.glossary_enabled = enabled;
        settings.auto_glossary = automatic;
        settings.glossary = glossary.entries.clone();
        e.enqueue_configured(
            &book.path,
            &[book.pages[0].id.clone()],
            &provider(&server),
            &settings,
        )
        .unwrap();
        e.start();
        let job = done(&e).await.remove(0);
        assert_eq!(job.status, "complete", "{:?}", job.error);
        assert!(!job.steps.iter().any(|s| s.stage == "glossary"));
        let calls = server.received_requests().await.unwrap();
        assert_eq!(calls.len(), 1, "No extra glossary request");
        let body: Value = serde_json::from_slice(&calls[0].body).unwrap();
        assert!(
            body["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("Translate current regions")
        );
        if !enabled {
            assert!(
                !body["messages"][1]["content"]
                    .to_string()
                    .contains("爱丽丝")
            );
        }
        assert_eq!(
            library::open(Path::new(&book.path))
                .unwrap()
                .glossary
                .entries,
            glossary.entries
        );
    }
}
#[test]
fn extraction_cache_keeps_empty_hits_and_bypasses_invalid_records() {
    let region = Region {
        source: "アリス".into(),
        target: "爱丽丝".into(),
        ..Default::default()
    };
    let empty = glossary::Extraction::default();
    assert_eq!(
        glossary::cached_candidates(
            &serde_json::to_string(&empty).unwrap(),
            std::slice::from_ref(&region)
        ),
        Some(empty)
    );
    for raw in [
        "{",
        "null",
        "{}",
        r#"{"terms":[{"regionId":"unknown","source":"アリス","target":"爱丽丝"}],"skipped":0}"#,
    ] {
        assert!(glossary::cached_candidates(raw, std::slice::from_ref(&region)).is_none());
    }
    let rows = glossary::Extraction {
        terms: vec![glossary::Candidate {
            region_id: region.id.clone(),
            source: "アリス".into(),
            target: "爱丽丝".into(),
        }],
        skipped: 2,
    };
    assert_eq!(
        glossary::cached_candidates(&serde_json::to_string(&rows).unwrap(), &[region]),
        Some(rows)
    );
}
#[tokio::test]
async fn untrusted_glossary_cache_cannot_insert_unrelated_terms() {
    let dir = tempfile::tempdir_in(root().join("test-output")).unwrap();
    let book = fixture(dir.path(), 1, 1, false);
    let server = server(&book, false, false).await;
    let mut profile = provider(&server);
    let settings = settings(false);
    profile.thinking_policy = Some(umanga_core::thinking::resolve(&profile).await);
    let hash = store::digest(
        &serde_json::to_vec(&(
            umanga_core::prompts::GLOSSARY_PROMPT_VERSION,
            &profile,
            &settings.source_language,
            &settings.target_language,
            &settings.glossary,
            "",
            book.pages[0]
                .regions
                .iter()
                .map(|r| (&r.id, &r.source))
                .collect::<Vec<_>>(),
        ))
        .unwrap(),
    );
    let bad = json!([{"regionId":"unknown","source":"Unrelated","target":"Injected"}]).to_string();
    for namespace in ["glossary:", "glossary:validated-v1:"] {
        store::connection(Path::new(&book.path))
            .unwrap()
            .execute(
                "INSERT INTO cache VALUES(?1,?2)",
                rusqlite::params![format!("{namespace}{hash}"), bad],
            )
            .unwrap();
    }
    let e = engine();
    e.enqueue_configured(&book.path, &[book.pages[0].id.clone()], &profile, &settings)
        .unwrap();
    e.start();
    let jobs = done(&e).await;
    assert_eq!(jobs[0].status, "complete", "{jobs:?}");
    let saved = library::open(Path::new(&book.path)).unwrap().glossary;
    assert!(
        saved
            .entries
            .iter()
            .all(|entry| entry.source != "Unrelated"),
        "cached terms require the same evidence as provider terms"
    );
    assert_eq!(saved.entries[0].source, "アリス");
}
#[tokio::test]
async fn local_pass_learns_before_translation_and_current_and_queued_pages_use_new_terms() {
    let dir = tempfile::tempdir_in(root().join("test-output")).unwrap();
    let book = fixture(dir.path(), 2, 7, false);
    let server = server(&book, false, false).await;
    let e = engine();
    e.enqueue_configured(
        &book.path,
        &book
            .pages
            .iter()
            .rev()
            .map(|p| p.id.clone())
            .collect::<Vec<_>>(),
        &provider(&server),
        &settings(false),
    )
    .unwrap();
    e.start();
    let jobs = done(&e).await;
    assert!(jobs.iter().all(|j| j.status == "complete"), "{jobs:?}");
    let glossary = library::open(Path::new(&book.path)).unwrap().glossary;
    assert_eq!(glossary.entries.len(), 1);
    assert_eq!(glossary.automatic_sources, vec!["アリス"]);
    assert!(
        jobs.iter()
            .all(|j| j.glossary_checkpoint.as_ref().unwrap().completed
                && j.settings.glossary[0].source == "アリス")
    );
    let calls = server.received_requests().await.unwrap();
    assert_eq!(calls.len(), 6);
    let bodies = calls
        .iter()
        .map(|r| serde_json::from_slice::<Value>(&r.body).unwrap())
        .collect::<Vec<_>>();
    for start in [0, 3] {
        assert!(
            bodies[start]["messages"][0]["content"]
                .as_str()
                .unwrap()
                .starts_with("识别并提取")
        );
        assert!(
            bodies[start + 1]["messages"][0]["content"]
                .as_str()
                .unwrap()
                .starts_with("识别并提取")
        );
        assert!(
            bodies[start + 2]["messages"][0]["content"]
                .as_str()
                .unwrap()
                .starts_with("Translate")
        );
    }
    assert!(
        !bodies[0]["messages"][1]["content"]
            .to_string()
            .contains("爱丽丝")
    );
    assert!(
        bodies[2]["messages"][1]["content"]
            .to_string()
            .contains("爱丽丝")
    );
    assert!(
        jobs.iter()
            .all(|j| j.glossary_checkpoint.as_ref().unwrap().skipped > 0)
    );
}
#[tokio::test]
async fn vision_reads_missing_sources_then_extracts_text_before_vision_translation() {
    let dir = tempfile::tempdir_in(root().join("test-output")).unwrap();
    let book = fixture(dir.path(), 1, 1, true);
    let server = server(&book, false, false).await;
    let e = engine();
    e.enqueue_configured(
        &book.path,
        &[book.pages[0].id.clone()],
        &provider(&server),
        &settings(true),
    )
    .unwrap();
    e.start();
    let job = done(&e).await.remove(0);
    assert_eq!(job.status, "complete", "{:?}", job.error);
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests
            .iter()
            .filter(|r| String::from_utf8_lossy(&r.body).contains("image_url"))
            .count(),
        2
    );
    assert!(
        job.steps
            .iter()
            .any(|s| s.stage == "translating" && s.status == "complete")
    );
    assert_eq!(
        store::page(Path::new(&book.path), &book.pages[0].id)
            .unwrap()
            .regions[0]
            .source,
        "アリスは城へ"
    );
}
#[tokio::test]
async fn vision_source_reading_is_conditional_and_failure_falls_back_without_late_extraction() {
    for scenario in ["disabled", "saved", "unreadable", "failed", "partial"] {
        let dir = tempfile::tempdir_in(root().join("test-output")).unwrap();
        let book = fixture(dir.path(), 1, 1, true);
        let book_path = Path::new(&book.path);
        let mut page = store::page(book_path, &book.pages[0].id).unwrap();
        if scenario == "saved" {
            page.regions[0].source = "アリスは城へ".into();
            store::save_page(book_path, &mut page, 0).unwrap();
        }
        let id = page.regions[0].id.clone();
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(move |r: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&r.body).unwrap();
            let system = body["messages"][0]["content"].as_str().unwrap();
            if system.starts_with("Transcribe") {
                assert!(["unreadable", "failed", "partial"].contains(&scenario));
                match scenario {
                    "failed" => text_response("malformed"),
                    "partial" => response(json!({"regions":[]})),
                    _ => response(json!({"regions":[{"id":id,"source":"","target":""}]})),
                }
            } else if system.starts_with("识别并提取") {
                assert_eq!(scenario, "saved");
                text_response("[1] アリス:爱丽丝")
            } else {
                response(json!({"regions":[{"id":id,"source":"アリスは城へ","target":"爱丽丝去了城堡"}]}))
            }
        }).mount(&server).await;
        let worker = engine();
        let mut settings = settings(true);
        if scenario == "disabled" {
            settings.auto_glossary = false;
        }
        worker
            .enqueue_configured(
                &book.path,
                &[page.id.clone()],
                &provider(&server),
                &settings,
            )
            .unwrap();
        worker.start();
        let job = done(&worker).await.remove(0);
        assert_eq!(job.status, "complete", "{scenario}: {:?}", job.error);
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), if scenario == "disabled" { 1 } else { 2 });
        assert!(
            String::from_utf8_lossy(&requests.last().unwrap().body)
                .contains("Read current image crops")
        );
        assert_eq!(
            store::page(book_path, &page.id).unwrap().regions[0].target,
            "爱丽丝去了城堡"
        );
        assert_eq!(
            job.steps
                .iter()
                .any(|s| s.stage == "glossary" && s.status == "warning"),
            ["failed", "partial"].contains(&scenario)
        );
    }
}
#[tokio::test]
async fn extraction_failure_warns_without_losing_translation() {
    let dir = tempfile::tempdir_in(root().join("test-output")).unwrap();
    let book = fixture(dir.path(), 1, 1, false);
    let server = server(&book, true, false).await;
    let e = engine();
    e.enqueue_configured(
        &book.path,
        &[book.pages[0].id.clone()],
        &provider(&server),
        &settings(false),
    )
    .unwrap();
    e.start();
    let job = done(&e).await.remove(0);
    assert_eq!(job.status, "complete");
    assert!(
        job.steps
            .iter()
            .any(|s| s.stage == "glossary" && s.status == "warning")
    );
    assert!(job.error.is_none());
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
    assert!(
        library::open(Path::new(&book.path))
            .unwrap()
            .glossary
            .entries
            .is_empty()
    );
}
#[tokio::test]
async fn retry_uses_saved_reading_and_glossary_without_repeating_requests() {
    let dir = tempfile::tempdir_in(root().join("test-output")).unwrap();
    let book = fixture(dir.path(), 1, 1, true);
    let server = server(&book, false, true).await;
    let e = engine();
    e.enqueue_configured(
        &book.path,
        &[book.pages[0].id.clone()],
        &provider(&server),
        &settings(true),
    )
    .unwrap();
    e.start();
    let first = done(&e).await.remove(0);
    assert_eq!(first.status, "failed");
    assert_eq!(first.stage, "translating");
    assert_eq!(server.received_requests().await.unwrap().len(), 3);
    server.reset().await;
    // Mount on the same endpoint captured by the durable job.
    let id = book.pages[0].regions[0].id.clone();
    Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(move |r: &wiremock::Request| {
        let body = String::from_utf8_lossy(&r.body);
        assert!(!body.contains("识别并提取") && !body.contains("Transcribe"));
        {
            assert!(body.contains("image_url"));
            response(json!({"regions":[{"id":id,"source":"アリスは城へ","target":"爱丽丝去了城堡"}]}))
        }
    }).mount(&server).await;
    let resumed = engine();
    resumed.recover(&book.path).unwrap();
    resumed.control(&first.id, "retry").unwrap();
    resumed.start();
    assert_eq!(done(&resumed).await[0].status, "complete");
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    assert_eq!(
        library::open(Path::new(&book.path))
            .unwrap()
            .glossary
            .entries
            .len(),
        1
    );
}
#[tokio::test]
async fn queued_language_changes_preserve_nonempty_captured_glossary() {
    for automatic in [false, true] {
        let dir = tempfile::tempdir_in(root().join("test-output")).unwrap();
        let book = fixture(dir.path(), 1, 1, false);
        let path = Path::new(&book.path);
        let e = engine();
        let server = server(&book, false, false).await;
        let mut settings = settings(false);
        settings.auto_glossary = automatic;
        settings.glossary = vec![GlossaryEntry {
            source: "アリス".into(),
            target: "原有中文".into(),
        }];
        settings.deepl_glossary_id = "captured-chinese-id".into();
        e.enqueue_configured(
            &book.path,
            &[book.pages[0].id.clone()],
            &provider(&server),
            &settings,
        )
        .unwrap();
        let mut current = library::open(path).unwrap().glossary;
        current.entries = vec![GlossaryEntry {
            source: "アリス".into(),
            target: "New English".into(),
        }];
        current.deepl_glossary_id = "new-english-id".into();
        library::save_glossary(dir.path(), path, current.revision, current).unwrap();
        let mut defaults = AppSettings::default();
        defaults.translation.target_language = "en".into();
        e.configure(&defaults);
        e.start();
        let job = done(&e).await.remove(0);
        assert_eq!(job.status, "complete", "{:?}", job.error);
        assert_eq!(job.settings.glossary, settings.glossary);
        assert_eq!(job.settings.deepl_glossary_id, "captured-chinese-id");
        let requests = server.received_requests().await.unwrap();
        let last = String::from_utf8(requests.last().unwrap().body.clone()).unwrap();
        assert!(last.contains("原有中文") && !last.contains("New English"));
    }
}
#[tokio::test]
async fn late_results_preserve_manual_terms_and_reject_language_changes_or_cancellation() {
    for change in ["manual", "language", "cancel"] {
        let dir = tempfile::tempdir_in(root().join("test-output")).unwrap();
        let book = fixture(dir.path(), 1, 1, false);
        let e = engine();
        let server = MockServer::builder().start().await;
        let id = book.pages[0].regions[0].id.clone();
        let book_path = PathBuf::from(&book.path);
        let library_root = dir.path().to_path_buf();
        let worker = e.clone();
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(move |request: &wiremock::Request| {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                if body["messages"][0]["content"]
                    .as_str()
                    .unwrap()
                    .starts_with("识别并提取")
                {
                    match change {
                        "manual" => {
                            let current = library::open(&book_path).unwrap();
                            let mut glossary = current.glossary.clone();
                            glossary.entries.push(GlossaryEntry {
                                source: "アリス".into(),
                                target: "手动名称".into(),
                            });
                            library::save_glossary(
                                &library_root,
                                &book_path,
                                current.glossary.revision,
                                glossary,
                            )
                            .unwrap();
                        }
                        "language" => {
                            let mut defaults = AppSettings::default();
                            defaults.translation.target_language = "en".into();
                            worker.configure(&defaults);
                            let current = library::open(&book_path).unwrap();
                            let mut glossary = current.glossary;
                            glossary.entries.push(GlossaryEntry { source: "アリス".into(), target: "New English".into() });
                            library::save_glossary(&library_root, &book_path, glossary.revision, glossary).unwrap();
                        }
                        "cancel" => {
                            worker.control(&worker.list()[0].id, "cancel").unwrap();
                        }
                        _ => unreachable!(),
                    }
                    text_response("[1] アリス:爱丽丝")
                } else {
                    response(json!({"regions":[{"id":id,"source":"アリスは城へ","target":"爱丽丝去了城堡"}]}))
                }
            })
            .mount(&server)
            .await;
        e.enqueue_configured(
            &book.path,
            &[book.pages[0].id.clone()],
            &provider(&server),
            &settings(false),
        )
        .unwrap();
        e.start();
        let job = done(&e).await.remove(0);
        let saved = library::open(Path::new(&book.path)).unwrap().glossary;
        assert!(saved.automatic_sources.is_empty());
        if change == "cancel" {
            assert_eq!(job.status, "cancelled");
            assert_eq!(server.received_requests().await.unwrap().len(), 1);
        } else {
            assert_eq!(job.status, "complete", "{:?}", job.error);
        }
        if change == "manual" {
            assert_eq!(saved.entries[0].target, "手动名称");
        } else if change == "language" {
            assert_eq!(saved.entries[0].target, "New English");
            let requests = server.received_requests().await.unwrap();
            let translation: Value =
                serde_json::from_slice(&requests.last().unwrap().body).unwrap();
            assert!(
                !translation.to_string().contains("New English"),
                "A captured Chinese job must not adopt an English glossary"
            );
        } else {
            assert!(saved.entries.is_empty());
        }
        if change == "language" {
            assert!(
                job.steps
                    .iter()
                    .any(|s| s.stage == "glossary" && s.status == "warning")
            );
        }
    }
}
#[tokio::test]
async fn saved_translation_retry_retains_glossary_warning_without_new_requests() {
    let dir = tempfile::tempdir_in(root().join("test-output")).unwrap();
    let book = fixture(dir.path(), 1, 1, false);
    let server = server(&book, true, false).await;
    let e = engine();
    e.enqueue_configured(
        &book.path,
        &[book.pages[0].id.clone()],
        &provider(&server),
        &settings(false),
    )
    .unwrap();
    e.start();
    let mut job = done(&e).await.remove(0);
    assert_eq!(job.status, "complete");
    // Simulate persisted failure after successful translation, before the final save completed.
    job.status = "failed".into();
    job.stage = "cleaning".into();
    store::save_job(Path::new(&book.path), &job).unwrap();
    server.reset().await;
    let resumed = engine();
    resumed.recover(&book.path).unwrap();
    resumed.control(&job.id, "retry").unwrap();
    resumed.start();
    let retried = done(&resumed).await.remove(0);
    assert_eq!(retried.status, "complete", "{:?}", retried.error);
    assert!(
        retried
            .steps
            .iter()
            .any(|step| step.stage == "glossary" && step.status == "warning")
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn existing_only_replies_preserve_manual_and_automatic_entries_without_revision_or_warning() {
    for automatic in [false, true] {
        let dir = tempfile::tempdir_in(root().join("test-output")).unwrap();
        let book = fixture(dir.path(), 1, 1, false);
        let book_path = Path::new(&book.path);
        let current = library::open(book_path).unwrap();
        let mut glossary = current.glossary;
        glossary.entries.push(GlossaryEntry {
            source: " アリス ".into(),
            target: "Saved preferred name".into(),
        });
        if automatic {
            glossary.automatic_sources.push("アリス".into());
        }
        library::save_glossary(dir.path(), book_path, glossary.revision, glossary).unwrap();
        let before = library::open(book_path).unwrap();
        let id = book.pages[0].regions[0].id.clone();
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(move |r: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&r.body).unwrap();
            let user = body["messages"][1]["content"].to_string();
            if body["messages"][0]["content"].as_str().unwrap().starts_with("识别并提取") {
                assert!(!user.contains("Saved preferred name") && !user.contains("Existing terms:"));
                text_response("[1] \" アリス \":\" 爱丽丝 \"\n[2] アリス:爱丽丝")
            } else {
                assert!(user.contains("Saved preferred name"), "Translation still receives its glossary");
                response(json!({"regions":[{"id":id,"source":"アリスは城へ","target":"爱丽丝去了城堡"}]}))
            }
        }).mount(&server).await;
        let worker = engine();
        worker
            .enqueue_configured(
                &book.path,
                &[book.pages[0].id.clone()],
                &provider(&server),
                &settings(false),
            )
            .unwrap();
        worker.start();
        let job = done(&worker).await.remove(0);
        assert_eq!(job.status, "complete", "{:?}", job.error);
        let checkpoint = job.glossary_checkpoint.unwrap();
        assert!(checkpoint.completed && checkpoint.warning.is_none());
        assert_eq!((checkpoint.added, checkpoint.skipped), (0, 0));
        assert!(job.steps.iter().any(|s| s.stage == "glossary"
            && s.status == "complete"
            && s.detail == "No new terms"));
        let after = library::open(book_path).unwrap();
        assert_eq!(after.revision, before.revision);
        assert_eq!(after.glossary.revision, before.glossary.revision);
        assert_eq!(after.glossary.entries, before.glossary.entries);
        assert_eq!(
            after.glossary.automatic_sources,
            before.glossary.automatic_sources
        );
        let cached: String = store::connection(book_path).unwrap().query_row("SELECT value FROM (SELECT data AS value FROM cache WHERE key LIKE 'glossary:validated-v3:%')", [], |r| r.get(0)).unwrap();
        let cached: glossary::Extraction = serde_json::from_str(&cached).unwrap();
        assert_eq!(
            cached.terms[0].target, "爱丽丝",
            "Cache precedes local book filtering"
        );
    }
}

#[tokio::test]
async fn mixed_replies_ignore_existing_sources_but_case_distinct_terms_are_added_once() {
    let dir = tempfile::tempdir_in(root().join("test-output")).unwrap();
    let book = fixture(dir.path(), 1, 1, false);
    let book_path = Path::new(&book.path);
    let mut page = store::page(book_path, &book.pages[0].id).unwrap();
    page.regions[0].source = "Alice ALICE Bob".into();
    store::save_page(book_path, &mut page, 0).unwrap();
    let current = library::open(book_path).unwrap();
    let mut glossary = current.glossary;
    glossary.entries.push(GlossaryEntry {
        source: " Alice ".into(),
        target: "Preferred Alice".into(),
    });
    library::save_glossary(dir.path(), book_path, glossary.revision, glossary).unwrap();
    let before = library::open(book_path).unwrap();
    let id = page.regions[0].id.clone();
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(move |r: &wiremock::Request| {
        let body: Value = serde_json::from_slice(&r.body).unwrap();
        if body["messages"][0]["content"].as_str().unwrap().starts_with("识别并提取") {
            text_response("[1] Alice:艾莉丝\n[2] ALICE:大写名\n[3] Bob:鲍勃\n[4] Bob:鲍勃")
        } else {
            response(json!({"regions":[{"id":id,"source":"Alice ALICE Bob","target":"艾莉丝 大写名 鲍勃"}]}))
        }
    }).mount(&server).await;
    let worker = engine();
    worker
        .enqueue_configured(&book.path, &[page.id], &provider(&server), &settings(false))
        .unwrap();
    worker.start();
    let job = done(&worker).await.remove(0);
    assert_eq!(job.status, "complete", "{:?}", job.error);
    let checkpoint = job.glossary_checkpoint.unwrap();
    assert_eq!((checkpoint.added, checkpoint.skipped), (2, 0));
    assert!(checkpoint.warning.is_none());
    let after = library::open(book_path).unwrap();
    assert_eq!(after.revision, before.revision + 1);
    assert_eq!(after.glossary.entries[0].target, "Preferred Alice");
    assert_eq!(after.glossary.automatic_sources, vec!["ALICE", "Bob"]);
}

#[tokio::test]
async fn extraction_cache_ignores_context_and_glossary_and_rechecks_current_saved_sources() {
    let dir = tempfile::tempdir_in(root().join("test-output")).unwrap();
    let book = fixture(dir.path(), 2, 1, false);
    let book_path = Path::new(&book.path);
    let mut preceding = store::page(book_path, &book.pages[0].id).unwrap();
    preceding.regions[0].source = "CONTEXT-ONLY-SENTINEL".into();
    store::save_page(book_path, &mut preceding, 0).unwrap();
    let server = MockServer::start().await;
    let id = book.pages[1].regions[0].id.clone();
    Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(move |r: &wiremock::Request| {
        let body: Value = serde_json::from_slice(&r.body).unwrap();
        if body["messages"][0]["content"].as_str().unwrap().starts_with("识别并提取") {
            assert!(!body.to_string().contains("CONTEXT-ONLY-SENTINEL"));
            text_response("[1] アリス:爱丽丝")
        } else {
            response(json!({"regions":[{"id":id,"source":"アリスは城へ","target":"爱丽丝去了城堡"}]}))
        }
    }).mount(&server).await;
    let profile = provider(&server);
    for round in 0..3 {
        if round > 0 {
            let mut current = library::open(book_path).unwrap();
            if round == 1 {
                current.glossary.entries[0].target = "Preferred manual name".into();
                current.glossary.automatic_sources.clear();
                current.glossary.entries.push(GlossaryEntry {
                    source: "UNSENT-EXISTING".into(),
                    target: "Sentinel translation".into(),
                });
            } else {
                current.glossary.entries.retain(|e| e.source != "アリス");
            }
            library::save_glossary(
                dir.path(),
                book_path,
                current.glossary.revision,
                current.glossary,
            )
            .unwrap();
            let mut page = store::page(book_path, &book.pages[1].id).unwrap();
            page.regions[0].target.clear();
            let revision = page.revision;
            store::save_page(book_path, &mut page, revision).unwrap();
        }
        let worker = engine();
        let mut options = settings(false);
        options.context_pages = if round == 0 { 0 } else { 1 };
        let submitted = worker
            .enqueue_configured(&book.path, &[book.pages[1].id.clone()], &profile, &options)
            .unwrap()
            .remove(0)
            .id;
        worker.start();
        let job = done(&worker)
            .await
            .into_iter()
            .find(|j| j.id == submitted)
            .unwrap();
        assert_eq!(job.status, "complete", "{:?}", job.error);
        assert_eq!(
            job.glossary_checkpoint.unwrap().added,
            usize::from(round != 1)
        );
        let saved = library::open(book_path).unwrap().glossary;
        assert_eq!(
            saved
                .entries
                .iter()
                .find(|e| e.source == "アリス")
                .unwrap()
                .target,
            if round == 1 {
                "Preferred manual name"
            } else {
                "爱丽丝"
            }
        );
    }
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| String::from_utf8_lossy(&r.body).contains("识别并提取"))
            .count(),
        1,
        "Changes to references reuse unfiltered extraction candidates"
    );
    assert!(
        requests
            .iter()
            .any(|r| String::from_utf8_lossy(&r.body).contains("CONTEXT-ONLY-SENTINEL")),
        "Ordinary translation still gets preceding-page context"
    );
    assert_eq!(
        store::connection(book_path)
            .unwrap()
            .query_row(
                "SELECT count(*) FROM cache WHERE key LIKE 'glossary:validated-v3:%'",
                [],
                |r| r.get::<_, usize>(0)
            )
            .unwrap(),
        1
    );
}
