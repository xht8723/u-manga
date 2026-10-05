use super::*;

#[tokio::test]
async fn simple_batches_checkpoint_before_failure_and_resume_without_repeating_success() {
    use serde_json::{Value, json};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
    let server = MockServer::start().await;
    Mock::given(path("/api/tags"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"models":[{"name":"simple-job","digest":"test","size":1}]})),
        )
        .mount(&server)
        .await;
    Mock::given(path("/api/show"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"capabilities":["completion"]})),
        )
        .mount(&server)
        .await;
    crate::ollama::details(&server.uri(), "simple-job", true).await;
    let first_count = Arc::new(AtomicUsize::new(0));
    let second_count = Arc::new(AtomicUsize::new(0));
    let fail = Arc::new(AtomicBool::new(true));
    let (first, second, fail_request) = (first_count.clone(), second_count.clone(), fail.clone());
    Mock::given(path("/v1/chat/completions")).respond_with(move |r: &wiremock::Request| {
        let body: Value=serde_json::from_slice(&r.body).unwrap();
        assert!(body.get("response_format").is_none()); assert_eq!(body["messages"].as_array().unwrap().len(),2);
        assert_eq!(body["messages"][0]["role"],"system");assert_eq!(body["messages"][0]["content"],"CAPTURED-SIMPLE");
        assert_eq!(body["messages"][1]["role"],"user");
        let prompt=body["messages"][1]["content"].as_str().unwrap();
        let target=if prompt.contains("Current texts:\n[1] 原文0") {
            first.fetch_add(1,Ordering::SeqCst);
            assert!(prompt.ends_with("[48] 原文47"));
            (1..=48).map(|i|format!("[{i}] 已保存")).collect::<Vec<_>>().join("\n")
        } else {
            assert!(prompt.ends_with("Current text:\n原文48"));
            second.fetch_add(1,Ordering::SeqCst);
            if fail_request.load(Ordering::SeqCst) { String::new() } else { "第二译文".into() }
        };
        assert!(!prompt.contains("CAPTURED-SIMPLE"));
        ResponseTemplate::new(200).set_body_json(json!({"choices":[{"message":{"role":"assistant","content":target},"finish_reason":"stop"}]}))
    }).mount(&server).await;
    let (_dir, book, engine) = fixture();
    let path = Path::new(&book.path);
    let mut page = book.pages[0].clone();
    page.regions = (0..50)
        .map(|i| Region {
            source: if i == 49 {
                "■■■■".into()
            } else {
                format!("原文{i}")
            },
            bbox: [10., 10., 110., 110.],
            overlay_only: true,
            ..Default::default()
        })
        .collect();
    store::save_page(path, &mut page, 0).unwrap();
    let original = store::file_hash(Path::new(&page.source.path)).unwrap();
    let mut profile = ProviderProfile {
        simple_translation: true,
        protocol: "ollama".into(),
        endpoint: server.uri(),
        model: "simple-job".into(),
        rate_limit: 100.,
        ..Default::default()
    };
    profile.instructions.simple_text_translation = Some("CAPTURED-SIMPLE".into());
    engine
        .enqueue_configured(&book.path, &[page.id.clone()], &profile, &settings())
        .unwrap();
    profile.simple_translation = false;
    profile.instructions.simple_text_translation = Some("EDITED-LATER".into());
    let (job, token) = engine.pick().unwrap();
    let id = job.id.clone();
    let result = engine.clone().process(&id, (job, token.clone())).await;
    assert!(result.is_err());
    engine.finish(&id, &token, result, 0);
    let checkpoint = store::page(path, &page.id).unwrap();
    assert!(
        checkpoint.regions[..48]
            .iter()
            .all(|r| r.target == "已保存")
    );
    assert!(checkpoint.regions[48..].iter().all(|r| r.target.is_empty()));
    assert_eq!(first_count.load(Ordering::SeqCst), 1);
    assert_eq!(second_count.load(Ordering::SeqCst), 1);
    let restored = store::jobs(path).unwrap().remove(0);
    assert!(restored.provider.simple_translation);
    assert_eq!(
        restored
            .provider
            .instructions
            .simple_text_translation
            .as_deref(),
        Some("CAPTURED-SIMPLE")
    );
    assert_eq!(restored.status, "failed");
    fail.store(false, Ordering::SeqCst);
    drop(engine);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let engine = Engine::new(
        _dir.path().join("no-models"),
        &root.join("assets/fonts"),
        Arc::new(|_| {}),
        Arc::new(|_| panic!("No hosted credentials")),
    )
    .unwrap();
    engine.recover(&book.path).unwrap();
    engine.control(&id, "retry").unwrap();
    let snapshot = engine.pick().unwrap();
    let token = snapshot.1.clone();
    let result = engine.clone().process(&id, snapshot).await;
    assert!(result.is_ok(), "{result:?}");
    engine.finish(&id, &token, result, 0);
    let saved = store::page(path, &page.id).unwrap();
    assert!(saved.regions[..48].iter().all(|r| r.target == "已保存"));
    assert_eq!(saved.regions[48].target, "第二译文");
    assert!(saved.regions[49].target.is_empty());
    assert!(saved.regions[49].review.is_some());
    assert_eq!(first_count.load(Ordering::SeqCst), 1);
    assert_eq!(second_count.load(Ordering::SeqCst), 2);
    assert_eq!(
        store::file_hash(Path::new(&page.source.path)).unwrap(),
        original
    );
}

#[tokio::test]
async fn translation_checkpoint_filters_empty_ocr_and_partial_cache_before_schema() {
    use serde_json::{Value, json};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    for all_empty in [false, true] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"models":[{"name":"filtered:1b","digest":"local","size":1}]}),
            ))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/show"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"capabilities":["completion"]})),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"choices":[{"message":{"role":"assistant","content":"{\"regions\":[{\"id\":\"pending\",\"source\":\"rewritten\",\"target\":\"你好\"}]}"},"finish_reason":"stop"}]}))).expect(if all_empty { 0 } else { 1 }).mount(&server).await;
        let (_dir, book, engine) = fixture();
        let book_path = Path::new(&book.path);
        let mut page = book.pages[0].clone();
        page.regions = ["cached", "empty", "pending"]
            .into_iter()
            .enumerate()
            .map(|(i, id)| Region {
                id: id.into(),
                source: if all_empty || id == "empty" {
                    " 　"
                } else {
                    "原文"
                }
                .into(),
                bbox: [10. + i as f32 * 35., 10., 40. + i as f32 * 35., 100.],
                overlay_only: true,
                ..Default::default()
            })
            .collect();
        store::save_page(book_path, &mut page, 0).unwrap();
        let original = documents::load(&page.source).unwrap();
        let original_hash = store::file_hash(Path::new(&page.source.path)).unwrap();
        let profile = ProviderProfile {
            protocol: "ollama".into(),
            simple_translation: false,
            endpoint: server.uri(),
            model: "filtered:1b".into(),
            ..Default::default()
        };
        // The OS can reuse a mock port previously cached by another test.
        crate::ollama::details(&profile.endpoint, &profile.model, true).await;
        let settings = settings();
        engine
            .enqueue_configured(&book.path, &[page.id.clone()], &profile, &settings)
            .unwrap();
        let (mut job, token) = engine.pick().unwrap();
        // Resume after successful local OCR, including its saved empty result.
        job.fresh = true;
        job.page_revision = Some(page.revision);
        job.record_step("detecting", "complete", "saved", None);
        job.record_step("ocr", "complete", "saved", None);
        page.fingerprint = store::digest(original.to_rgb8().as_raw());
        if !all_empty {
            let context = store::preceding_context(book_path, page.number, 0)
                .unwrap()
                .cache_identity()
                .unwrap();
            let mut pending = page.clone();
            pending
                .regions
                .retain(|r| crate::safety::has_text(&r.source));
            let key = request_cache_key(
                store::cache_key(&pending, &settings, &profile, &context).unwrap(),
                &job,
            );
            store::cache_put_validated(
                book_path,
                &key,
                &[TranslationItem {
                    id: "cached".into(),
                    source: "原文".into(),
                    target: "缓存".into(),
                    direction: String::new(),
                }],
            )
            .unwrap();
        }
        let id = job.id.clone();
        let result = engine.clone().process(&id, (job, token.clone())).await;
        assert!(result.is_ok(), "{result:?}");
        engine.finish(&id, &token, result, 0);
        let saved = store::page(book_path, &page.id).unwrap();
        assert_eq!(saved.status, "review");
        let rendered = image::open(saved.rendered.as_ref().unwrap())
            .unwrap()
            .to_rgb8();
        // Missing translation must retain actual original pixels, not just a warning.
        let original_pixels = original.to_rgb8();
        if all_empty {
            assert_eq!(rendered, original_pixels);
        } else {
            for y in 20..95 {
                for x in 50..65 {
                    assert_eq!(rendered.get_pixel(x, y), original_pixels.get_pixel(x, y));
                }
            }
        }
        assert_eq!(
            store::file_hash(Path::new(&page.source.path)).unwrap(),
            original_hash
        );
        for (before, after) in page.regions.iter().zip(&saved.regions) {
            assert_eq!(before.source, after.source);
        }
        let requests = server.received_requests().await.unwrap();
        let chats: Vec<_> = requests
            .iter()
            .filter(|r| r.url.path() == "/v1/chat/completions")
            .collect();
        assert_eq!(chats.len(), if all_empty { 0 } else { 1 });
        if !all_empty {
            // Partial cache coverage must not shrink the request's source context.
            // The new partial provider reply is still useful; only its missing ID stays blank.
            assert!(saved.regions[0].target.is_empty());
            assert_eq!(saved.regions[2].target, "你好");
            let body: Value = serde_json::from_slice(&chats[0].body).unwrap();
            let array = &body["response_format"]["json_schema"]["schema"]["properties"]["regions"];
            assert_eq!(array["minItems"], 2);
            assert_eq!(array["maxItems"], 2);
            assert_eq!(
                array["items"]["properties"]["id"]["enum"],
                json!(["cached", "pending"])
            );
        }
        for region in saved
            .regions
            .iter()
            .filter(|r| all_empty || r.id == "empty")
        {
            assert!(region.target.is_empty());
            assert!(
                region
                    .review
                    .as_deref()
                    .unwrap()
                    .contains("original preserved")
            );
        }
        assert!(
            engine.list()[0]
                .steps
                .iter()
                .any(|s| s.stage == "translating" && s.status == "warning")
        );
    }
}

fn fixture() -> (tempfile::TempDir, Project, Arc<Engine>) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = tempfile::tempdir_in(root.join("test-output")).unwrap();
    let source = dir.path().join("原稿.png");
    let mut image = image::RgbImage::from_pixel(120, 120, image::Rgb([255, 255, 255]));
    for y in 25..75 {
        for x in 40..46 {
            image.put_pixel(x, y, image::Rgb([0, 0, 0]));
        }
    }
    image.save(&source).unwrap();
    let mut pages = documents::import(&[source.to_string_lossy().into()]).unwrap();
    pages[0].regions = vec![Region {
        bbox: [25., 20., 95., 100.],
        source: "既存".into(),
        target: "已有".into(),
        ..Default::default()
    }];
    let book = store::create(&dir.path().join("本.umanga"), "Manual", &pages).unwrap();
    let engine = Engine::new(
        dir.path().join("no-models"),
        &root.join("assets/fonts"),
        Arc::new(|_| {}),
        Arc::new(|_| panic!("Preparation must never access credentials or providers")),
    )
    .unwrap();
    (dir, book, engine)
}

#[tokio::test]
async fn simple_batch_publication_requires_a_complete_reply_or_exact_complete_cache() {
    use serde_json::{Value, json};
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
    for scenario in ["complete-cache", "partial-cache", "malformed", "cancel"] {
        let server = MockServer::start().await;
        Mock::given(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"models":[{"name":"batch-cache","digest":"fixture","size":1}]}),
            ))
            .mount(&server)
            .await;
        Mock::given(path("/api/show"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"capabilities":["completion"]})),
            )
            .mount(&server)
            .await;
        crate::ollama::details(&server.uri(), "batch-cache", true).await;
        let seen = Arc::new(tokio::sync::Notify::new());
        let notify = seen.clone();
        Mock::given(path("/v1/chat/completions")).respond_with(move |r: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&r.body).unwrap();
            assert_eq!(body["messages"][0]["role"], "system");
            assert_eq!(body["messages"][1]["role"], "user");
            let prompt = body["messages"][1]["content"].as_str().unwrap();
            assert!(prompt.ends_with("Current texts:\n[1] 原文一\n[2] 原文二"));
            notify.notify_one();
            let reply = if scenario == "malformed" { "[1] 不应保存" } else { "[1] 新译一\n[2] 新译二" };
            ResponseTemplate::new(200).set_delay(std::time::Duration::from_millis(if scenario == "cancel" { 200 } else { 0 }))
                .set_body_json(json!({"choices":[{"message":{"role":"assistant","content":reply},"finish_reason":"stop"}]}))
        }).expect(if scenario == "complete-cache" { 0 } else { 1 }).mount(&server).await;
        let (_dir, book, engine) = fixture();
        let path = Path::new(&book.path);
        let mut page = book.pages[0].clone();
        page.regions = ["原文一", "原文二"]
            .into_iter()
            .map(|source| Region {
                source: source.into(),
                bbox: [10., 10., 110., 110.],
                overlay_only: true,
                ..Default::default()
            })
            .collect();
        store::save_page(path, &mut page, 0).unwrap();
        page.fingerprint = store::digest(documents::load(&page.source).unwrap().to_rgb8().as_raw());
        let profile = ProviderProfile {
            protocol: "ollama".into(),
            endpoint: server.uri(),
            model: "batch-cache".into(),
            simple_translation: true,
            rate_limit: 100.,
            ..Default::default()
        };
        let s = settings();
        let context = store::preceding_context(path, page.number, 0)
            .unwrap()
            .cache_identity()
            .unwrap();
        let key = store::cache_key(&page, &s, &profile, &context).unwrap();
        if scenario.ends_with("cache") {
            let count = if scenario == "complete-cache" { 2 } else { 1 };
            let cached: Vec<_> = page.regions[..count]
                .iter()
                .map(|r| TranslationItem {
                    id: r.id.clone(),
                    source: r.source.clone(),
                    target: "缓存译文".into(),
                    direction: String::new(),
                })
                .collect();
            store::cache_put_validated(path, &key, &cached).unwrap();
        }
        engine
            .enqueue_configured(&book.path, &[page.id.clone()], &profile, &s)
            .unwrap();
        let (job, token) = engine.pick().unwrap();
        let id = job.id.clone();
        let owned_engine = engine.clone();
        let owned_id = id.clone();
        let worker_token = token.clone();
        let worker =
            tokio::spawn(async move { owned_engine.process(&owned_id, (job, worker_token)).await });
        if scenario == "cancel" {
            tokio::time::timeout(std::time::Duration::from_secs(5), seen.notified())
                .await
                .unwrap();
            engine.control_all("stop").unwrap();
        }
        let result = worker.await.unwrap();
        assert_eq!(
            result.is_ok(),
            scenario.ends_with("cache"),
            "{scenario}: {result:?}"
        );
        engine.finish(&id, &token, result, 0);
        let saved = store::page(path, &page.id).unwrap();
        let targets: Vec<_> = saved.regions.iter().map(|r| r.target.as_str()).collect();
        match scenario {
            "complete-cache" => assert_eq!(targets, ["缓存译文", "缓存译文"]),
            "partial-cache" => assert_eq!(targets, ["新译一", "新译二"]),
            _ => {
                assert_eq!(targets, ["", ""]);
                assert!(
                    store::cache_validated(path, &key, &page.regions, true)
                        .unwrap()
                        .is_none()
                );
            }
        }
        for (before, after) in page.regions.iter().zip(&saved.regions) {
            assert_eq!(before.source, after.source);
            assert_eq!(before.bbox, after.bbox);
        }
    }
}
fn settings() -> TranslationSettings {
    let mut s = TranslationSettings::default();
    s.cleanup.method = CleanupMethod::Solid;
    s
}

#[tokio::test]
async fn fresh_translation_failure_before_detection_preserves_everything() {
    let (dir, book, engine) = fixture();
    let mut page = book.pages[0].clone();
    page.background = Some(page.source.path.clone());
    store::save_page(Path::new(&book.path), &mut page, 0).unwrap();
    let s = settings();
    assert!(
        engine
            .enqueue_fresh(
                &book.path,
                &page.id,
                page.revision,
                false,
                &ProviderProfile::default(),
                &s,
                dir.path()
            )
            .is_err()
    );
    let job = engine
        .enqueue_fresh(
            &book.path,
            &page.id,
            page.revision,
            true,
            &ProviderProfile::default(),
            &s,
            dir.path(),
        )
        .unwrap()
        .remove(0);
    assert!(job.fresh);
    let snapshot = engine.pick().unwrap();
    let token = snapshot.1.clone();
    let result = engine.clone().process(&job.id, snapshot).await;
    assert!(result.is_err());
    engine.finish(&job.id, &token, result, 0);
    assert_eq!(
        serde_json::to_value(store::page(Path::new(&book.path), &page.id).unwrap()).unwrap(),
        serde_json::to_value(page).unwrap()
    );
}

#[tokio::test]
async fn region_reservation_survives_stop_and_excludes_edit_submit_and_dispatch() {
    let (dir, book, engine) = fixture();
    let page = &book.pages[0];
    let s = settings();
    let id = uid();
    let guard = engine.reserve_region(&id, &book.path, &page.id).unwrap();
    assert!(
        engine
            .enqueue(
                &book.path,
                std::slice::from_ref(&page.id),
                &ProviderProfile::default()
            )
            .is_err()
    );
    assert!(engine.pause_for_edit(&book.path, &page.id).await.is_err());
    assert!(engine.reserve_book(&book.path).await.is_err());
    engine.control_all("stop").unwrap();
    assert!(guard.cancel.is_cancelled());
    engine.control_all("start").unwrap();
    assert!(engine.reserve_region(&uid(), &book.path, &page.id).is_err());
    drop(guard);
    let next = engine.reserve_region(&uid(), &book.path, &page.id).unwrap();
    drop(next);
    assert_eq!(
        engine
            .enqueue_preparation(&book.path, &page.id, page.revision, true, &s, dir.path())
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn local_region_read_failure_does_not_access_service_or_write_page() {
    let (dir, book, engine) = fixture();
    let page = &book.pages[0];
    let s = settings();
    let request = RegionRequest {
        id: uid(),
        path: book.path.clone(),
        page_id: page.id.clone(),
        expected: page.revision,
        region: page.regions[0].clone(),
    };
    let guard = engine
        .reserve_region(&request.id, &book.path, &page.id)
        .unwrap();
    let result = engine
        .process_region(
            request,
            RegionAction::Read,
            s,
            ProviderProfile::default(),
            dir.path().join("absent"),
            guard,
            Arc::new(|_| {}),
        )
        .await;
    assert!(result.is_err());
    assert_eq!(
        serde_json::to_value(store::page(Path::new(&book.path), &page.id).unwrap()).unwrap(),
        serde_json::to_value(page).unwrap()
    );
    assert!(store::jobs(Path::new(&book.path)).unwrap().is_empty());
}

#[tokio::test]
async fn simple_region_results_are_drafts_and_reject_cancelled_stale_or_unreadable_input() {
    use serde_json::json;
    use std::time::Duration;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
    for scenario in ["success", "empty", "unreadable", "cancel", "stale"] {
        let server = MockServer::start().await;
        Mock::given(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"models":[{"name":"simple-region","digest":"test","size":1}]}),
            ))
            .mount(&server)
            .await;
        Mock::given(path("/api/show"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"capabilities":["completion"]})),
            )
            .mount(&server)
            .await;
        crate::ollama::details(&server.uri(), "simple-region", true).await;
        let received = Arc::new(tokio::sync::Notify::new());
        let seen = received.clone();
        Mock::given(path("/v1/chat/completions")).respond_with(move |r: &wiremock::Request| {
            let body:serde_json::Value=serde_json::from_slice(&r.body).unwrap();
            assert_eq!(body["messages"].as_array().unwrap().len(),2);
            assert_eq!(body["messages"][0]["role"],"system");assert_eq!(body["messages"][1]["role"],"user");
            assert!(body["messages"][1]["content"].as_str().unwrap().ends_with("Current text:\n手動訂正"));
            seen.notify_one();
            ResponseTemplate::new(200).set_delay(Duration::from_millis(200)).set_body_json(json!({"choices":[{"message":{"role":"assistant","content":if scenario == "empty" { "，。" } else { "阿·布=朋友，草稿译文。" }},"finish_reason":"stop"}]}))
        }).expect(if scenario=="unreadable"{0}else{1}).mount(&server).await;
        let (dir, book, engine) = fixture();
        let mut page = book.pages[0].clone();
        let mut region = page.regions[0].clone();
        region.source = if scenario == "unreadable" {
            "■■■■"
        } else {
            "手動訂正"
        }
        .into();
        let id = uid();
        let guard = engine.reserve_region(&id, &book.path, &page.id).unwrap();
        let request = RegionRequest {
            id,
            path: book.path.clone(),
            page_id: page.id.clone(),
            expected: page.revision,
            region,
        };
        let p = ProviderProfile {
            simple_translation: true,
            protocol: "ollama".into(),
            endpoint: server.uri(),
            model: "simple-region".into(),
            rate_limit: 100.,
            ..Default::default()
        };
        let e = engine.clone();
        let root = dir.path().to_owned();
        let pending = tokio::spawn(async move {
            e.process_region(
                request,
                RegionAction::Translate,
                settings(),
                p,
                root,
                guard,
                Arc::new(|_| {}),
            )
            .await
        });
        if scenario != "unreadable" {
            tokio::time::timeout(Duration::from_secs(5), received.notified())
                .await
                .unwrap();
        }
        if scenario == "cancel" {
            engine.control_all("stop").unwrap();
        }
        if scenario == "stale" {
            page.regions[0].target = "newer saved edit".into();
            store::save_page(Path::new(&book.path), &mut page, 0).unwrap();
        }
        let result = pending.await.unwrap();
        if scenario == "success" {
            assert_eq!(result.unwrap().text, "阿·布=朋友\n草稿译文");
        } else {
            assert!(result.is_err(), "{scenario}");
        }
        assert_eq!(
            serde_json::to_value(store::page(Path::new(&book.path), &page.id).unwrap()).unwrap(),
            serde_json::to_value(&page).unwrap()
        );
        assert!(store::jobs(Path::new(&book.path)).unwrap().is_empty());
    }
}

#[test]
fn fresh_cache_namespace_reuses_only_its_own_requests() {
    let (dir, book, engine) = fixture();
    let page = &book.pages[0];
    let fresh = engine
        .enqueue_fresh(
            &book.path,
            &page.id,
            0,
            true,
            &ProviderProfile::default(),
            &settings(),
            dir.path(),
        )
        .unwrap()
        .remove(0);
    let key = request_cache_key("input".into(), &fresh);
    assert_ne!(key, "input");
    assert_eq!(request_cache_key("input".into(), &fresh), key);
    let mut other = fresh.clone();
    other.id = uid();
    assert_ne!(request_cache_key("input".into(), &other), key);
    other.fresh = false;
    assert_eq!(request_cache_key("input".into(), &other), "input");
}

#[tokio::test]
async fn detected_reset_is_atomic_and_fresh_resume_keeps_new_regions_and_text() {
    let (dir, book, engine) = fixture();
    let mut page = book.pages[0].clone();
    page.background = Some(page.source.path.clone());
    page.rendered = Some(page.source.path.clone());
    let original = std::fs::read(&page.source.path).unwrap();
    store::save_page(Path::new(&book.path), &mut page, 0).unwrap();
    let job = engine
        .enqueue_fresh(
            &book.path,
            &page.id,
            page.revision,
            true,
            &ProviderProfile::default(),
            &settings(),
            dir.path(),
        )
        .unwrap()
        .remove(0);
    let (_, token) = engine.pick().unwrap();
    let mut revision = page.revision;
    let detected = Region {
        bbox: [25., 20., 95., 100.],
        ..Default::default()
    };
    let id = detected.id.clone();
    replace_detected(&mut page, vec![detected], true, false);
    // Reset remains in-memory until the detection/page checkpoint commits together.
    assert!(
        store::page(Path::new(&book.path), &page.id)
            .unwrap()
            .background
            .is_some()
    );
    engine
        .checkpoint(
            &job.id,
            &mut page,
            &mut revision,
            "detecting",
            "complete",
            "1 region",
        )
        .unwrap();
    assert!(page.background.is_none() && page.rendered.is_none() && page.cleanup.is_none());
    assert!(page.regions[0].source.is_empty() && page.regions[0].target.is_empty());
    page.regions[0].source = "fresh source".into();
    page.regions[0].target = "新译文".into();
    engine
        .checkpoint(
            &job.id,
            &mut page,
            &mut revision,
            "translating",
            "complete",
            "1 region",
        )
        .unwrap();
    engine.finish(
        &job.id,
        &token,
        Err(anyhow::anyhow!("Interrupted before rendering")),
        0,
    );
    engine.control(&job.id, "retry").unwrap();
    let snapshot = engine.pick().unwrap();
    let token = snapshot.1.clone();
    let result = engine.clone().process(&job.id, snapshot).await;
    assert!(result.is_ok(), "{result:?}");
    engine.finish(&job.id, &token, result, 1);
    let saved = store::page(Path::new(&book.path), &page.id).unwrap();
    assert_eq!(saved.regions[0].id, id);
    assert_eq!(saved.regions[0].target, "新译文");
    assert!(saved.background.is_none());
    assert_eq!(std::fs::read(&page.source.path).unwrap(), original);
}
#[tokio::test]
async fn cancelled_worker_keeps_page_reserved_until_exit() {
    let (_dir, book, engine) = fixture();
    let ids = [book.pages[0].id.clone()];
    engine
        .enqueue(&book.path, &ids, &ProviderProfile::default())
        .unwrap();
    let (job, token) = engine.pick().unwrap();
    engine.control(&job.id, "cancel").unwrap();
    assert!(token.is_cancelled());
    assert!(
        engine
            .enqueue(&book.path, &ids, &ProviderProfile::default())
            .unwrap()
            .is_empty()
    );
    assert!(engine.pick().is_none());
    engine.finish(&job.id, &token, Err(anyhow::anyhow!("Cancelled")), 0);
    assert_eq!(
        engine
            .enqueue(&book.path, &ids, &ProviderProfile::default())
            .unwrap()
            .len(),
        1
    );
}
#[tokio::test]
async fn book_reservation_unwinds_after_failed_operation() {
    let (_dir, book, engine) = fixture();
    let reservation = engine.reserve_book(&book.path).await.unwrap();
    assert!(
        engine
            .enqueue(
                &book.path,
                &[book.pages[0].id.clone()],
                &ProviderProfile::default()
            )
            .is_err()
    );
    drop(reservation);
    assert_eq!(
        engine
            .enqueue(
                &book.path,
                &[book.pages[0].id.clone()],
                &ProviderProfile::default()
            )
            .unwrap()
            .len(),
        1
    );
}
#[tokio::test]
async fn preparation_requires_confirmation_and_detection_failure_preserves_edits() {
    let (dir, book, engine) = fixture();
    let page = &book.pages[0];
    let hash = std::fs::read(&page.source.path).unwrap();
    assert!(
        engine
            .enqueue_preparation(
                &book.path,
                &page.id,
                page.revision,
                false,
                &settings(),
                dir.path()
            )
            .is_err()
    );
    assert!(
        engine
            .enqueue_preparation(
                &book.path,
                &page.id,
                page.revision + 1,
                true,
                &settings(),
                dir.path()
            )
            .is_err()
    );
    engine
        .enqueue_preparation(
            &book.path,
            &page.id,
            page.revision,
            true,
            &settings(),
            dir.path(),
        )
        .unwrap();
    let snapshot = engine.pick().unwrap();
    let id = snapshot.0.id.clone();
    let token = snapshot.1.clone();
    let result = engine.clone().process(&id, snapshot).await;
    assert!(
        result.is_err(),
        "The detector was deliberately not installed"
    );
    engine.finish(&id, &token, result, 0);
    let saved = store::page(Path::new(&book.path), &page.id).unwrap();
    assert_eq!(
        serde_json::to_value(&saved).unwrap(),
        serde_json::to_value(page).unwrap()
    );
    assert_eq!(std::fs::read(&page.source.path).unwrap(), hash);
}
#[tokio::test]
async fn preparation_resumes_checkpoints_without_reset_or_ocr_and_stays_untranslated() {
    let (dir, book, engine) = fixture();
    let mut page = book.pages[0].clone();
    engine
        .enqueue_preparation(&book.path, &page.id, 0, true, &settings(), dir.path())
        .unwrap();
    let (job, token) = engine.pick().unwrap();
    page.regions[0].prepared = true;
    page.regions[0].target.clear();
    page.regions[0].source.clear();
    let mut revision = 0;
    engine
        .checkpoint(
            &job.id,
            &mut page,
            &mut revision,
            "detecting",
            "complete",
            "1 region",
        )
        .unwrap();
    // Even an empty recognized result is a saved OCR checkpoint, not a reason to load another model on retry.
    engine
        .checkpoint(
            &job.id,
            &mut page,
            &mut revision,
            "ocr",
            "complete",
            "1 region",
        )
        .unwrap();
    engine.finish(
        &job.id,
        &token,
        Err(anyhow::anyhow!("Interrupted after OCR")),
        2,
    );
    let recovered = Engine::new(
        dir.path().join("still-no-models"),
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts"),
        Arc::new(|_| {}),
        Arc::new(|_| panic!("No secrets")),
    )
    .unwrap();
    recovered.recover(&book.path).unwrap();
    recovered.control(&job.id, "resume").unwrap();
    let snapshot = recovered.pick().unwrap();
    let token = snapshot.1.clone();
    let result = recovered.clone().process(&job.id, snapshot).await;
    assert!(result.is_ok(), "{result:?}");
    recovered.finish(&job.id, &token, result, 2);
    let saved = store::page(Path::new(&book.path), &page.id).unwrap();
    assert_eq!(saved.regions[0].id, page.regions[0].id);
    assert!(saved.regions[0].prepared);
    assert!(saved.regions[0].target.is_empty());
    assert!(saved.cleanup.is_some());
    assert_eq!(saved.status, "prepared");
    assert!(
        store::translated_page_ids(Path::new(&book.path))
            .unwrap()
            .is_empty()
    );
    assert!(
        store::cleanup_page_ids(Path::new(&book.path))
            .unwrap()
            .contains(&page.id)
    );
    let count: i64 = store::connection(Path::new(&book.path))
        .unwrap()
        .query_row("SELECT COUNT(*) FROM cache", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
    // A later confirmed edit makes an old preparation retry stale.
    let mut edited = saved.clone();
    edited.regions[0].target = "manual".into();
    store::save_page(Path::new(&book.path), &mut edited, saved.revision).unwrap();
    let mut stale = job;
    stale.page_revision = Some(saved.revision);
    assert!(
        recovered
            .clone()
            .process(&stale.id.clone(), (stale, CancellationToken::new()))
            .await
            .unwrap_err()
            .to_string()
            .contains("Page changed")
    );
}
#[test]
fn monotonic_timer_survives_stopping_retry_and_recovery_without_idle_time() {
    let (dir, book, engine) = fixture();
    let page = &book.pages[0];
    let jobs = engine
        .enqueue_preparation(&book.path, &page.id, 0, true, &settings(), dir.path())
        .unwrap();
    assert_eq!(engine.snapshot(None)[0].job.elapsed_ms, 0);
    let (job, token) = engine.pick().unwrap();
    engine.state.lock().jobs.get_mut(&job.id).unwrap().clock =
        Some((Instant::now() - std::time::Duration::from_secs(3), 0));
    engine.timer_checkpoint();
    let elapsed = store::jobs(Path::new(&book.path)).unwrap()[0].elapsed_ms;
    assert!((3000..3500).contains(&elapsed));
    engine.control_all("stop").unwrap();
    let stopping = engine.snapshot(None).remove(0);
    assert!(stopping.stopping && stopping.timer_running);
    engine.state.lock().jobs.get_mut(&job.id).unwrap().clock =
        Some((Instant::now() - std::time::Duration::from_secs(4), 0));
    engine.finish(&job.id, &token, Err(anyhow::anyhow!("Cancelled")), 0);
    let paused = engine.snapshot(None).remove(0);
    assert!(!paused.timer_running);
    assert!((4000..4500).contains(&paused.job.elapsed_ms));
    let accumulated = paused.job.elapsed_ms;
    engine.control_all("start").unwrap();
    assert_eq!(engine.snapshot(None)[0].job.elapsed_ms, accumulated);
    let (again, token) = engine.pick().unwrap();
    assert_eq!(again.id, jobs[0].id);
    engine.state.lock().jobs.get_mut(&job.id).unwrap().clock = Some((
        Instant::now() - std::time::Duration::from_secs(2),
        accumulated,
    ));
    engine.finish(&job.id, &token, Err(anyhow::anyhow!("Injected failure")), 0);
    let failed = engine.snapshot(None).remove(0);
    assert!(!failed.timer_running);
    assert!(failed.job.elapsed_ms >= accumulated + 2000);
    let reboot = fixture().2;
    reboot.recover(&book.path).unwrap();
    assert_eq!(
        reboot.snapshot(None)[0].job.elapsed_ms,
        failed.job.elapsed_ms
    );
    assert!(!reboot.snapshot(None)[0].timer_running);
}
#[tokio::test]
async fn dispatch_rechecks_captured_requirements_and_pauses_without_processing() {
    let (dir, book, engine) = fixture();
    let page = &book.pages[0];
    engine
        .enqueue_preparation(&book.path, &page.id, 0, true, &settings(), dir.path())
        .unwrap();
    engine.set_requirements(Arc::new(|job| {
        Box::pin(async move {
            assert_eq!(job.kind, JobKind::Preparation);
            bail!("Missing required pack")
        })
    }));
    let snapshot = engine.pick().unwrap();
    let id = snapshot.0.id.clone();
    let token = snapshot.1.clone();
    let result = engine.clone().process(&id, snapshot).await;
    assert!(result.is_err());
    engine.finish(&id, &token, result, 0);
    assert_eq!(engine.snapshot(None)[0].job.status, "paused");
    assert!(
        engine.snapshot(None)[0]
            .job
            .error
            .as_ref()
            .unwrap()
            .contains("Missing required pack")
    );
    assert_eq!(
        store::page(Path::new(&book.path), &page.id)
            .unwrap()
            .revision,
        0
    );
}
