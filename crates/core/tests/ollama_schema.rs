use serde_json::{Value, json};
#[path = "support/generation.rs"]
mod generation;
use std::{path::PathBuf, sync::Arc, time::Duration};
use umanga_core::{documents, ollama, pipeline::Engine, providers, store, types::*};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

async fn server() -> (MockServer, ProviderProfile) {
    let server = MockServer::builder().start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "models": [{"name": "schema-test:4b", "digest": "schema-test", "size": 100}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/show"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "capabilities": ["completion", "vision", "thinking"],
            "thinking": {"values": [false, true], "default": true}
        })))
        .mount(&server)
        .await;
    let profile = ProviderProfile {
        simple_translation: false,
        protocol: "ollama".into(),
        endpoint: server.uri(),
        model: "schema-test:4b".into(),
        vision: false, // Ollama uses discovered capabilities, not this hosted-model flag.
        ..Default::default()
    };
    // Each test owns an unpooled endpoint for the lifetime of its Tokio runtime.
    // Refresh still exercises the actual metadata contract.
    ollama::details(&profile.endpoint, &profile.model, true).await;
    (server, profile)
}

fn completion(content: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "id": "mock", "model": "schema-test:4b",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": content}, "finish_reason": "stop"}]
    }))
}

async fn reply(server: &MockServer, content: &str) {
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(completion(content))
        .expect(1)
        .mount(server)
        .await;
}

fn regions() -> Vec<Region> {
    ["local-a", "local-b"]
        .iter()
        .map(|id| Region {
            id: (*id).into(),
            source: "こんにちは".into(),
            bbox: [0., 0., 48., 48.],
            ..Default::default()
        })
        .collect()
}

#[tokio::test]
async fn background_vision_encoding_preserves_tiles_schema_and_thinking() {
    let (server, p) = server().await;
    let mut region = regions().remove(0);
    region.bbox = [0., 0., 1600., 80.];
    let image = image::DynamicImage::new_rgb8(1600, 80);
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(completion(
            r#"{"regions":[{"id":"local-a","source":"原文","target":""}]}"#,
        ))
        .expect(2)
        .mount(&server)
        .await;
    let settings = TranslationSettings::default();
    let original = providers::transcribe(&p, "", &settings, &region, &image)
        .await
        .unwrap();
    let prepared = providers::prepare_vision_region(image, region.clone(), Default::default())
        .await
        .unwrap();
    assert_eq!(
        providers::transcribe_prepared(&p, "", &settings, &region, &prepared)
            .await
            .unwrap(),
        original
    );
    let bodies = chat_bodies(&server).await;
    assert_eq!(
        bodies[0], bodies[1],
        "Encoding must not change request pixels, tile order, prompts, schema or Thinking"
    );
    let token = tokio_util::sync::CancellationToken::new();
    token.cancel();
    assert!(
        providers::prepare_vision_region(image::DynamicImage::new_rgb8(1, 1), region, token)
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("Cancelled")
    );
    assert_eq!(
        chat_bodies(&server).await.len(),
        2,
        "Cancelled encoding sends no request"
    );
}

#[tokio::test]
async fn vision_transcription_has_its_own_schema_and_keeps_empty_sources() {
    let (server, p) = server().await;
    let r = regions().remove(0);
    reply(
        &server,
        r#"{"regions":[{"id":"local-a","source":"","target":""}]}"#,
    )
    .await;
    let text = providers::transcribe(
        &p,
        "",
        &TranslationSettings::default(),
        &r,
        &image::DynamicImage::new_rgb8(48, 48),
    )
    .await
    .unwrap();
    assert!(text.is_empty());
    let body = chat_bodies(&server).await.remove(0);
    let schema = &body["response_format"]["json_schema"]["schema"];
    assert_eq!(
        schema["properties"]["regions"]["items"]["properties"]["target"]["const"],
        ""
    );
    assert_eq!(schema["properties"]["regions"]["minItems"], 1);
    let system = body["messages"][0]["content"].as_str().unwrap();
    assert!(system.contains("Do not translate"));
    assert!(!system.contains("Translate manga from"));
    assert!(body.to_string().contains("image_url"));
}

#[tokio::test]
async fn region_translation_uses_corrected_source_without_images_or_page_writes() {
    use umanga_core::pipeline::{RegionAction, RegionRequest};
    let (server, p) = server().await;
    reply(
        &server,
        r#"{"regions":[{"id":"local-a","source":"修正した原文","target":"新的译文"}]}"#,
    )
    .await;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = tempfile::tempdir_in(root.join("test-output")).unwrap();
    let source = dir.path().join("original.png");
    image::DynamicImage::new_rgb8(48, 48).save(&source).unwrap();
    let mut pages = documents::import(&[source.to_string_lossy().into()]).unwrap();
    pages[0].regions = vec![regions().remove(0)];
    pages[0].regions[0].target = "old".into();
    let book = store::create(&dir.path().join("book.umanga"), "Region", &pages).unwrap();
    let original = serde_json::to_value(&book.pages[0]).unwrap();
    let engine = Engine::new(
        dir.path().join("no-models"),
        &root.join("assets/fonts"),
        Arc::new(|_| {}),
        Arc::new(|_| panic!("Ollama must be keyless")),
    )
    .unwrap();
    let s = TranslationSettings {
        mode: "vision".into(),
        ..Default::default()
    };
    let mut r = book.pages[0].regions[0].clone();
    r.source = "修正した原文".into();
    let request = RegionRequest {
        id: uid(),
        path: book.path.clone(),
        page_id: book.pages[0].id.clone(),
        expected: 0,
        region: r,
    };
    let guard = engine
        .reserve_region(&request.id, &request.path, &request.page_id)
        .unwrap();
    let result = engine
        .process_region(
            request,
            RegionAction::Translate,
            s,
            p,
            dir.path().join("absent"),
            guard,
            Arc::new(|_| {}),
        )
        .await
        .unwrap();
    assert_eq!(result.text, "新的译文");
    assert_eq!(
        serde_json::to_value(
            store::page(PathBuf::from(&book.path).as_path(), &result.page_id).unwrap()
        )
        .unwrap(),
        original
    );
    assert!(
        store::jobs(PathBuf::from(&book.path).as_path())
            .unwrap()
            .is_empty()
    );
    let body = chat_bodies(&server).await.remove(0);
    let raw = body.to_string();
    assert!(!raw.contains("image_url"));
    assert!(raw.contains("修正した原文"));
    assert!(raw.contains("exact supplied source"));
}

#[tokio::test]
async fn stop_cancels_active_region_transport_without_writing_or_releasing_early() {
    use umanga_core::pipeline::{RegionAction, RegionRequest};
    let (server, p) = server().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            completion(r#"{"regions":[{"id":"local-a","source":"こんにちは","target":"late"}]}"#)
                .set_delay(Duration::from_secs(5)),
        )
        .expect(1)
        .mount(&server)
        .await;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = tempfile::tempdir_in(root.join("test-output")).unwrap();
    let source = dir.path().join("original.png");
    image::DynamicImage::new_rgb8(48, 48).save(&source).unwrap();
    let original_bytes = std::fs::read(&source).unwrap();
    let mut pages = documents::import(&[source.to_string_lossy().into()]).unwrap();
    pages[0].regions = vec![regions().remove(0)];
    let book = store::create(&dir.path().join("book.umanga"), "Cancel region", &pages).unwrap();
    let baseline = serde_json::to_value(&book.pages[0]).unwrap();
    let engine = Engine::new(
        dir.path().join("no-models"),
        &root.join("assets/fonts"),
        Arc::new(|_| {}),
        Arc::new(|_| panic!("No credential lookup")),
    )
    .unwrap();
    let s = TranslationSettings::default();
    let request = RegionRequest {
        id: uid(),
        path: book.path.clone(),
        page_id: pages[0].id.clone(),
        expected: 0,
        region: pages[0].regions[0].clone(),
    };
    let guard = engine
        .reserve_region(&request.id, &request.path, &request.page_id)
        .unwrap();
    let worker = engine.clone();
    let directory = dir.path().join("no-models");
    let task = tokio::spawn(async move {
        worker
            .process_region(
                request,
                RegionAction::Translate,
                s,
                p,
                directory,
                guard,
                Arc::new(|_| {}),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while chat_bodies(&server).await.is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    engine.hold();
    let result = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    assert!(result.unwrap_err().to_string().contains("Cancelled"));
    assert_eq!(
        serde_json::to_value(store::page(std::path::Path::new(&book.path), &pages[0].id).unwrap())
            .unwrap(),
        baseline
    );
    assert_eq!(std::fs::read(&source).unwrap(), original_bytes);
    assert!(
        store::jobs(std::path::Path::new(&book.path))
            .unwrap()
            .is_empty()
    );
    assert_eq!(chat_bodies(&server).await.len(), 1);
}

#[tokio::test]
async fn large_vision_regions_keep_labeled_tiles_and_reference_data_separate() {
    let (server, p) = server().await;
    let mut r = regions().remove(0);
    r.bbox = [0., 0., 1600., 60.];
    r.source.clear();
    reply(
        &server,
        r#"{"regions":[{"id":"local-a","source":"こんにちは","target":"你好"}]}"#,
    )
    .await;
    let image = image::DynamicImage::new_rgb8(1600, 60);
    providers::llm(
        &p,
        "",
        &TranslationSettings::default(),
        &[r],
        Some(&image),
        "Page 1:\n前の原文",
    )
    .await
    .unwrap();
    let body = chat_bodies(&server).await.remove(0);
    let system = body["messages"][0]["content"].as_str().unwrap();
    assert!(!system.contains("前の原文"));
    assert!(system.contains("Read current image crops"));
    let content = body["messages"][1]["content"].as_array().unwrap();
    assert!(
        content[0]["text"]
            .as_str()
            .unwrap()
            .contains("Page 1:\n前の原文")
    );
    assert_eq!(
        content.iter().filter(|p| p["type"] == "image_url").count(),
        2
    );
    let labels: Vec<_> = content
        .iter()
        .filter_map(|p| p["text"].as_str())
        .filter(|t| t.contains("tile row"))
        .collect();
    assert_eq!(labels.len(), 2);
    assert!(labels[0].contains("column 1/2"));
    assert!(labels[1].contains("column 2/2"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn all_page_batches_share_source_context_and_completed_pages_stay_unchanged() {
    let (server, mut profile) = server().await;
    profile.rate_limit = 20.;
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(|request: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let ids = body["response_format"]["json_schema"]["schema"]["properties"]["regions"]["items"]["properties"]["id"]["enum"].as_array().unwrap();
            completion(&json!({"regions":ids.iter().map(|id|json!({"id":id,"source":"こんにちは","target":"你好"})).collect::<Vec<_>>()} ).to_string())
        }).expect(3).mount(&server).await;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let folder = tempfile::tempdir_in(root.join("test-output")).unwrap();
    let source = folder.path().join("original.png");
    image::RgbImage::from_pixel(200, 200, image::Rgb([255, 255, 255]))
        .save(&source)
        .unwrap();
    let mut previous = documents::import(&[source.to_string_lossy().into()])
        .unwrap()
        .remove(0);
    previous.regions = vec![Region {
        source: "姉は城に向かった。".into(),
        target: "PREVIOUS-TARGET-DO-NOT-SEND".into(),
        bbox: [5., 5., 100., 100.],
        ..Default::default()
    }];
    let mut current = previous.clone();
    current.id = uid();
    current.number = 1;
    current.regions = (0..97)
        .map(|_| Region {
            source: "こんにちは".into(),
            bbox: [5., 5., 195., 195.],
            overlay_only: true,
            ..Default::default()
        })
        .collect();
    let book_path = folder.path().join("batch.umanga");
    let book = store::create(
        &book_path,
        "Batch fixture",
        &[previous.clone(), current.clone()],
    )
    .unwrap();
    let settings = TranslationSettings {
        context_pages: 1,
        cleanup: CleanupSettings {
            method: CleanupMethod::Solid,
            ..Default::default()
        },
        ..Default::default()
    };
    let engine = Engine::new(
        root.join("assets/models"),
        &root.join("assets/fonts"),
        Arc::new(|_| {}),
        Arc::new(|_| anyhow::bail!("No credentials for local test")),
    )
    .unwrap();
    engine
        .enqueue_configured(&book.path, &[current.id.clone()], &profile, &settings)
        .unwrap();
    engine.start();
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let job = engine.list().remove(0);
            assert_ne!(job.status, "failed", "{:?}", job.error);
            if job.status == "complete" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    let bodies = chat_bodies(&server).await;
    let mut references = vec![];
    let mut counts = vec![];
    for body in bodies {
        let array = &body["response_format"]["json_schema"]["schema"]["properties"]["regions"];
        assert_eq!(array["minItems"], array["maxItems"]);
        let user = body["messages"][1]["content"].as_str().unwrap();
        let reference = user
            .split("Current regions to translate:")
            .next()
            .unwrap()
            .to_owned();
        assert!(reference.contains("Page 1:\n姉は城に向かった。"));
        assert!(!user.contains("PREVIOUS-TARGET-DO-NOT-SEND"));
        assert!(
            !reference.contains("你好"),
            "Earlier batches must not become context"
        );
        references.push(reference);
        counts.push(
            body["response_format"]["json_schema"]["schema"]["properties"]["regions"]["maxItems"]
                .as_u64()
                .unwrap(),
        );
    }
    assert_eq!(counts, vec![48, 48, 1]);
    assert!(references.iter().all(|v| v == &references[0]));
    assert_eq!(
        serde_json::to_value(store::page(&book_path, &previous.id).unwrap()).unwrap(),
        serde_json::to_value(previous).unwrap()
    );
    assert_eq!(
        store::page(&book_path, &current.id)
            .unwrap()
            .regions
            .iter()
            .filter(|r| !r.target.is_empty())
            .count(),
        97
    );
}

async fn chat_bodies(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/v1/chat/completions")
        .map(|r| {
            assert_eq!(r.headers["authorization"], "Bearer ollama");
            let body = serde_json::from_slice(&r.body).unwrap();
            generation::assert_service_defaults(&body, "ollama");
            body
        })
        .collect()
}

#[tokio::test]
async fn text_and_vision_send_the_exact_batch_schema_on_the_actual_transport() {
    for vision in [false, true] {
        let (server, p) = server().await;
        let regions = regions();
        let schema = providers::translation_schema(&regions);
        reply(&server, r#"{"regions":[{"id":"local-b","source":"こんにちは","target":"Hello"},{"id":"local-a","source":"こんにちは","target":"Hi","direction":"vertical","bbox":[0,0,9999,9999],"strokes":[{}]}]}"#).await;
        let s = TranslationSettings {
            mode: if vision { "vision" } else { "local" }.into(),
            source_language: "ja".into(),
            target_language: "en".into(),
            glossary_enabled: true,
            glossary: vec![GlossaryEntry {
                source: "太郎".into(),
                target: "Taro".into(),
            }],
            ..Default::default()
        };
        let image = image::DynamicImage::new_rgb8(48, 48);
        let items = providers::llm(
            &p,
            "MUST-NOT-BE-SENT",
            &s,
            &regions,
            vision.then_some(&image),
            "Earlier dialogue",
        )
        .await
        .unwrap();
        assert_eq!(
            items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            ["local-b", "local-a"]
        );
        assert!(items.iter().all(|i| i.direction.is_empty()));
        let bodies = chat_bodies(&server).await;
        assert_eq!(bodies.len(), 1);
        let body = &bodies[0];
        assert_eq!(body["model"], p.model);
        assert_eq!(body["stream"], false);
        assert!(body.get("temperature").is_none());
        assert_eq!(body["reasoning_effort"], "none");
        assert!(body.get("format").is_none());
        assert_eq!(
            body["response_format"],
            json!({
                "type": "json_schema",
                "json_schema": {"name": "umanga_regions", "strict": true, "schema": schema}
            })
        );
        let prompt = body["messages"][0]["content"].as_str().unwrap();
        assert!(prompt.contains("Japanese (ja)") && prompt.contains("English (en)"));
        assert!(!prompt.contains("Taro") && !prompt.contains("Earlier dialogue"));
        assert!(!prompt.contains("Response JSON Schema"));
        let user = body["messages"][1].to_string();
        assert!(user.contains("Taro") && user.contains("Earlier dialogue"));
        assert!(user.contains("Use previous dialogue as context; translate only current text."));
        assert_eq!(user.contains("data:image/png;base64,"), vision);
        assert!(user.contains("local-a") && user.contains("local-b"));
        assert!(!body.to_string().contains("MUST-NOT-BE-SENT"));
    }
}

#[tokio::test]
async fn transcription_keeps_schema_and_glossary_omits_schema_without_generation_overrides() {
    for glossary in [false, true] {
        let (server, mut p) = server().await;
        p.thinking = true;
        let regions = regions();
        if glossary {
            reply(&server, "NONE").await;
            assert!(
                providers::extract_glossary(&p, "", &TranslationSettings::default(), &regions)
                    .await
                    .unwrap()
                    .terms
                    .is_empty()
            );
        } else {
            reply(&server, r#"{"regions":[{"id":"local-a","source":"こんにちは","target":""},{"id":"local-b","source":"こんにちは","target":""}]}"#).await;
            let items = providers::transcribe_batch(
                &p,
                "",
                &TranslationSettings::default(),
                &regions,
                &image::DynamicImage::new_rgb8(48, 48),
            )
            .await
            .unwrap();
            assert_eq!(items.len(), 2);
        }
        let bodies = chat_bodies(&server).await;
        assert_eq!(bodies.len(), 1);
        let body = &bodies[0];
        assert_eq!(body["model"], p.model);
        assert_ne!(body["reasoning_effort"], "none");
        assert!(body["reasoning_effort"].is_string());
        if glossary {
            assert!(body.get("response_format").is_none());
        } else {
            assert_eq!(body["response_format"]["type"], "json_schema");
            assert_eq!(body["response_format"]["json_schema"]["strict"], true);
            assert!(
                body["response_format"]["json_schema"]["schema"]["properties"]
                    .get("regions")
                    .is_some()
            );
        }
        assert_eq!(
            body.to_string().contains("data:image/png;base64,"),
            !glossary
        );
    }
}

#[test]
fn schema_closes_objects_requires_strings_and_escapes_exact_ids() {
    let mut regions = regions();
    regions[1].id = "id-\"\\\n日本語".into();
    let schema = providers::translation_schema(&regions);
    assert_eq!(schema["required"], json!(["regions"]));
    assert_eq!(schema["additionalProperties"], false);
    let array = &schema["properties"]["regions"];
    assert_eq!(array["type"], "array");
    assert_eq!(array["minItems"], 2);
    assert_eq!(array["maxItems"], 2);
    let row = &array["items"];
    assert_eq!(row["additionalProperties"], false);
    assert_eq!(row["required"], json!(["id", "source", "target"]));
    assert_eq!(row["properties"].as_object().unwrap().len(), 3);
    for field in ["id", "source", "target"] {
        assert_eq!(row["properties"][field]["type"], "string");
    }
    assert_eq!(row["properties"]["target"], json!({"type":"string"}));
    assert_eq!(
        row["properties"]["id"]["enum"],
        json!([regions[0].id, regions[1].id])
    );
    let serialized = serde_json::to_string(&schema).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&serialized).unwrap(), schema);
    let response = json!({"regions":[{"id":regions[1].id,"source":"a","target":"b"}]}).to_string();
    assert_eq!(
        providers::validate(&response, &regions).unwrap()[0].id,
        regions[1].id
    );
}

#[tokio::test]
async fn cardinality_uses_only_readable_submitted_sources_and_empty_input_never_calls() {
    for count in [1, 3, 6] {
        let (server, p) = server().await;
        let mut input: Vec<_> = (0..count)
            .map(|i| Region {
                id: format!("readable-{i}"),
                source: "原文".into(),
                ..Default::default()
            })
            .collect();
        input.insert(
            0,
            Region {
                id: "empty".into(),
                ..Default::default()
            },
        );
        input.push(Region {
            id: "whitespace".into(),
            source: " \n\t　".into(),
            ..Default::default()
        });
        reply(&server, r#"{"regions":[]}"#).await;
        assert!(
            providers::llm(&p, "", &TranslationSettings::default(), &input, None, "")
                .await
                .unwrap()
                .is_empty()
        );
        let body = chat_bodies(&server).await.remove(0);
        let schema = &body["response_format"]["json_schema"]["schema"];
        let array = &schema["properties"]["regions"];
        assert_eq!(array["minItems"], count);
        assert_eq!(array["maxItems"], count);
        assert_eq!(
            array["items"]["properties"]["id"]["enum"],
            json!(
                (0..count)
                    .map(|i| format!("readable-{i}"))
                    .collect::<Vec<_>>()
            )
        );
        assert!(
            !body["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("Response JSON Schema")
        );
    }
    let (server, p) = server().await;
    let input = vec![
        Region::default(),
        Region {
            source: " \n　".into(),
            ..Default::default()
        },
    ];
    let before = server.received_requests().await.unwrap().len();
    assert!(
        providers::llm(&p, "", &TranslationSettings::default(), &input, None, "")
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(server.received_requests().await.unwrap().len(), before);
}

#[test]
fn partial_recovery_still_enforces_text_resource_limits() {
    for (field, limit) in [
        ("source", umanga_core::safety::MAX_SOURCE_CHARS),
        ("target", umanga_core::safety::MAX_TARGET_CHARS),
    ] {
        let mut row = json!({"id":"local-a","source":"original","target":"Hi"});
        row[field] = json!("界".repeat(limit + 1));
        let error =
            providers::validate(&json!({"regions":[row]}).to_string(), &regions()).unwrap_err();
        assert!(error.to_string().contains("render budget"));
    }
}

#[tokio::test]
async fn region_retranslation_without_usable_target_preserves_existing_text() {
    use umanga_core::pipeline::{RegionAction, RegionRequest};
    for content in [
        r#"{"regions":[]}"#,
        r#"{"regions":[{"id":"local-a","source":"rewritten","target":"  "}]}"#,
    ] {
        let (server, p) = server().await;
        reply(&server, content).await;
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let dir = tempfile::tempdir_in(root.join("test-output")).unwrap();
        let source = dir.path().join("original.png");
        image::DynamicImage::new_rgb8(48, 48).save(&source).unwrap();
        let mut pages = documents::import(&[source.to_string_lossy().into()]).unwrap();
        let mut region = regions().remove(0);
        region.target = "existing translation".into();
        pages[0].regions = vec![region.clone()];
        let book = store::create(&dir.path().join("book.umanga"), "Unresolved", &pages).unwrap();
        let before = serde_json::to_value(&book.pages[0]).unwrap();
        region.target = "unsaved manual draft".into();
        let request = RegionRequest {
            id: uid(),
            path: book.path.clone(),
            page_id: pages[0].id.clone(),
            expected: 0,
            region: region.clone(),
        };
        let engine = Engine::new(
            dir.path().join("no-models"),
            &root.join("assets/fonts"),
            Arc::new(|_| {}),
            Arc::new(|_| panic!("No keys")),
        )
        .unwrap();
        let settings = TranslationSettings::default();
        let guard = engine
            .reserve_region(&request.id, &request.path, &request.page_id)
            .unwrap();
        let error = engine
            .process_region(
                request,
                RegionAction::Translate,
                settings,
                p,
                dir.path().into(),
                guard,
                Arc::new(|_| {}),
            )
            .await
            .err()
            .unwrap();
        assert!(
            error
                .to_string()
                .contains("No translation returned for this region; existing text preserved")
        );
        assert_eq!(region.target, "unsaved manual draft");
        assert_eq!(
            serde_json::to_value(
                store::page(std::path::Path::new(&book.path), &pages[0].id).unwrap()
            )
            .unwrap(),
            before
        );
        let body = chat_bodies(&server).await.remove(0);
        assert_eq!(
            body["response_format"]["json_schema"]["schema"]["properties"]["regions"]["minItems"],
            1
        );
    }
}

#[tokio::test]
async fn malformed_and_untrusted_output_fails_without_repair_or_retry() {
    for content in [
        "not JSON",
        "{\"regions\":[",
        "{}",
        r#"{"regions":{}}"#,
        r#"{"regions":[null]}"#,
        r#"{"regions":[{"id":12,"source":"a","target":"b"}]}"#,
        r#"{"regions":[{"source":"a","target":"b"}]}"#,
        r#"{"regions":[{"id":"invented","target":""}]}"#,
        r#"{"regions":[{"id":"local-a","target":""},{"id":"local-a","target":"Hi"}]}"#,
        r#"{"regions":[{"id":"local-a","target":""},{"id":"local-a","target":""}]}"#,
        r#"{"regions":[{"id":"local-a","source":42,"target":"Hi"}]}"#,
        r#"{"regions":[{"id":"local-a","source":null,"target":""}]}"#,
        r#"{"regions":[{"id":"local-a","target":["Hi"]}]}"#,
        r#"{"regions":[{"id":"local-a","target":null}]}"#,
    ] {
        let (server, p) = server().await;
        reply(&server, content).await;
        assert!(
            providers::llm(
                &p,
                "",
                &TranslationSettings::default(),
                &regions(),
                None,
                ""
            )
            .await
            .is_err(),
            "{content}"
        );
        assert_eq!(chat_bodies(&server).await.len(), 1);
    }
}

#[tokio::test]
async fn partial_and_unreadable_output_remains_partial() {
    for (content, count) in [
        (r#"{"regions":[]}"#, 0),
        (
            r#"{"regions":[{"id":"local-a","source":"","target":"   "}]}"#,
            0,
        ),
        (
            r#"{"regions":[{"id":"local-a","source":"original","target":"Hi"}]}"#,
            1,
        ),
        (
            r#"{"regions":[{"id":"local-a","source":"original","target":"Hi"},{"id":"local-b","source":"","target":""}]}"#,
            1,
        ),
        // A nonconforming server omitting text retains the existing partial contract.
        (
            r#"{"regions":[{"id":"local-a","target":"Hi"},{"id":"local-b"}]}"#,
            1,
        ),
    ] {
        let (server, p) = server().await;
        reply(&server, content).await;
        let items = providers::llm(
            &p,
            "",
            &TranslationSettings::default(),
            &regions(),
            None,
            "",
        )
        .await
        .unwrap();
        assert_eq!(items.len(), count);
        if count > 0 {
            assert_eq!(items[0].id, "local-a");
            assert_eq!(items[0].target, "Hi");
        }
        assert_eq!(chat_bodies(&server).await.len(), 1);
    }
}

#[tokio::test]
async fn unsupported_schema_is_not_retried_with_weaker_constraints() {
    let (server, p) = server().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({"error": {"message": "unsupported response_format"}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let error = providers::llm(
        &p,
        "",
        &TranslationSettings::default(),
        &regions(),
        None,
        "",
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("structured-output support"));
    assert_eq!(chat_bodies(&server).await.len(), 1);
}

#[tokio::test]
async fn reasoning_is_not_a_translation_and_empty_or_truncated_answers_are_actionable() {
    let valid = r#"{"regions":[{"id":"local-a","source":"こんにちは","target":"Hello"}]}"#;
    for (content, reasoning, finish, expected) in [
        (
            "",
            "private reasoning with source text",
            "length",
            "context or output limit",
        ),
        ("", valid, "stop", "reasoning but no final translation"),
        ("   ", "", "stop", "empty final answer"),
        ("{\"regions\":[", "", "length", "context or output limit"),
        // Even a syntactically closed partial response is not a completed answer.
        (valid, "", "length", "context or output limit"),
    ] {
        let (server, p) = server().await;
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices":[{"message":{"role":"assistant","content":content,"reasoning":reasoning},"finish_reason":finish}],
                "usage":{"prompt_tokens":333,"completion_tokens":3763,"total_tokens":4096}
            }))).expect(1).mount(&server).await;
        let error = providers::llm(
            &p,
            "",
            &TranslationSettings::default(),
            &regions(),
            None,
            "",
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains(expected), "{error}");
        assert!(!error.contains("private reasoning") && !error.contains(valid));
        assert_eq!(chat_bodies(&server).await.len(), 1);
    }
}

#[tokio::test]
async fn a_final_answer_is_used_even_if_a_server_still_returns_reasoning() {
    let (server, p) = server().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices":[{"message":{"role":"assistant","content":r#"{"regions":[{"id":"local-a","source":"こんにちは","target":"Hello"}]}"#,"reasoning":"not the answer"},"finish_reason":"stop"}]
        }))).expect(1).mount(&server).await;
    let items = providers::llm(
        &p,
        "",
        &TranslationSettings::default(),
        &regions(),
        None,
        "",
    )
    .await
    .unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].target, "Hello");
}

#[tokio::test]
async fn empty_batches_do_not_send_an_empty_enum_or_load_a_model() {
    let server = MockServer::builder().start().await;
    let p = ProviderProfile {
        simple_translation: false,
        protocol: "ollama".into(),
        endpoint: server.uri(),
        ..Default::default()
    };
    assert!(
        providers::llm(&p, "", &TranslationSettings::default(), &[], None, "")
            .await
            .unwrap()
            .is_empty()
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_schema_uses_captured_settings_and_preserves_local_layout_and_missing_text() {
    let (server, mut p) = server().await;
    reply(&server, r#"{"regions":[{"id":"local-a","source":"rewritten","target":"Hello","direction":"vertical","bbox":[0,0,9999,9999],"allowFill":true},{"id":"local-b","source":"unreadable","target":""}]}"#).await;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = tempfile::tempdir_in(root.join("test-output")).unwrap();
    let image_path = dir.path().join("synthetic.png");
    image::RgbImage::from_pixel(200, 200, image::Rgb([255, 255, 255]))
        .save(&image_path)
        .unwrap();
    let mut page = documents::import(&[image_path.to_string_lossy().into()])
        .unwrap()
        .remove(0);
    page.regions = ["local-a", "local-b", "omitted"]
        .iter()
        .enumerate()
        .map(|(i, id)| Region {
            id: (*id).into(),
            source: "こんにちは".into(),
            bbox: [5., 5. + i as f32 * 60., 180., 55. + i as f32 * 60.],
            direction: if i == 0 { "horizontal" } else { "auto" }.into(),
            overlay_only: true,
            ..Default::default()
        })
        .collect();
    let path = dir.path().join("book.umanga");
    let book = store::create(&path, "Schema fixture", &[page.clone()]).unwrap();
    let mut settings = TranslationSettings {
        mode: "local".into(),
        target_language: "en".into(),
        ..Default::default()
    };
    settings.cleanup.method = CleanupMethod::Solid;
    let engine = Engine::new(
        root.join("assets/models"),
        &root.join("assets/fonts"),
        Arc::new(|_| {}),
        Arc::new(|_| anyhow::bail!("Ollama must not look up credentials")),
    )
    .unwrap();
    engine
        .enqueue_configured(&book.path, &[page.id.clone()], &p, &settings)
        .unwrap();
    settings.target_language = "fr".into();
    settings.mode = "vision".into();
    store::settings(&path, &settings).unwrap();
    p.model = "changed-after-enqueue".into();
    engine.start();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let job = engine.list().remove(0);
            assert_ne!(job.status, "failed", "{:?}", job.error);
            if job.status == "complete" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let saved = store::page(&path, &page.id).unwrap();
    assert_eq!(saved.status, "review");
    assert_eq!(saved.regions[0].target, "Hello");
    for (before, after) in page.regions.iter().zip(&saved.regions) {
        assert_eq!(after.id, before.id);
        assert_eq!(after.source, before.source);
        assert_eq!(after.bbox, before.bbox);
        assert_eq!(after.direction, before.direction);
        assert_eq!(after.allow_fill, before.allow_fill);
    }
    for r in &saved.regions[1..] {
        assert!(r.target.is_empty());
        assert!(r.review.as_deref().unwrap().contains("original preserved"));
    }
    let bodies = chat_bodies(&server).await;
    assert_eq!(bodies.len(), 1);
    assert_eq!(bodies[0]["model"], "schema-test:4b");
    assert!(
        bodies[0]["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("from Japanese (ja) to English (en)")
    );
    assert!(!bodies[0].to_string().contains("data:image"));
    assert_eq!(
        bodies[0]["response_format"]["json_schema"]["schema"],
        providers::translation_schema(&page.regions)
    );
}

#[test]
fn isolated_endpoints_survive_unrelated_connection_owner_shutdown() {
    use std::sync::mpsc;
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }
    async fn request(profile: &ProviderProfile) -> anyhow::Result<()> {
        providers::llm(
            profile,
            "",
            &TranslationSettings::default(),
            &regions(),
            None,
            "",
        )
        .await?;
        Ok(())
    }
    let host = runtime();
    let ((owner_server, owner_profile), (borrower_server, borrower_profile)) = host.block_on(async {
        let owner = server().await;
        let borrower = server().await;
        assert_ne!(owner.0.uri(), borrower.0.uri(), "Test endpoints must not share a pooled driver");
        let content = r#"{"regions":[{"id":"local-a","source":"こんにちは","target":"a"},{"id":"local-b","source":"こんにちは","target":"b"}]}"#;
        reply(&owner.0, content).await;
        Mock::given(method("POST")).and(path("/v1/chat/completions"))
            .respond_with(completion(content).set_delay(Duration::from_millis(300)))
            .expect(1).mount(&borrower.0).await;
        (owner, borrower)
    });
    let (ready_tx, ready_rx) = mpsc::channel();
    let (close_tx, close_rx) = tokio::sync::oneshot::channel::<()>();
    let owner = std::thread::spawn(move || {
        let rt = runtime();
        rt.block_on(async {
            let result = request(&owner_profile).await.map_err(|e| e.to_string());
            ready_tx.send(result).unwrap();
            let _ = close_rx.await;
        });
    });
    let first = ready_rx.recv_timeout(Duration::from_secs(15)).unwrap();
    if let Err(error) = first {
        let _ = close_tx.send(());
        owner.join().unwrap();
        panic!("Owner request failed: {error}");
    }
    let borrower = std::thread::spawn(move || runtime().block_on(request(&borrower_profile)));
    let received = host.block_on(async {
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if borrower_server
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .any(|r| r.url.path() == "/v1/chat/completions")
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
    });
    let _ = close_tx.send(());
    owner.join().unwrap();
    received.expect("Borrower must reach its endpoint before owner teardown");
    borrower
        .join()
        .unwrap()
        .expect("An unrelated runtime cannot own this endpoint's driver");
    assert_eq!(host.block_on(chat_bodies(&owner_server)).len(), 1);
    assert_eq!(host.block_on(chat_bodies(&borrower_server)).len(), 1);
}
