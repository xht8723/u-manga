use tokio_util::sync::CancellationToken;
use umanga_core::{models, providers, store, types::*};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};
#[tokio::test]
async fn openai_transport_keeps_local_ids_and_rejects_bad_responses() {
    let server = MockServer::builder().start().await;
    let profile = ProviderProfile {
        simple_translation: false,
        endpoint: format!("{}/v1", server.uri()),
        model: "test".into(),
        ..Default::default()
    };
    let region = Region {
        id: "trusted-local-id".into(),
        source: "こんにちは".into(),
        ..Default::default()
    };
    let payload = serde_json::json!({"regions":[{"id":region.id,"source":"こんにちは","target":"你好","direction":"vertical","bbox":[0,0,9999,9999]}]});
    Mock::given(method("POST")).and(path("/v1/chat/completions")).and(header("authorization","Bearer dummy-test-key")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id":"mock","model":"test","choices":[{"index":0,"message":{"role":"assistant","content":payload.to_string()},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}))).expect(1).mount(&server).await;
    let items = providers::llm(
        &profile,
        "dummy-test-key",
        &TranslationSettings::default(),
        &[region],
        None,
        "",
    )
    .await
    .unwrap();
    assert_eq!(items[0].id, "trusted-local-id");
    assert_eq!(items[0].target, "你好");
    assert!(items[0].direction.is_empty());
    let requests = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        body["reasoning_effort"], "none",
        "Unknown hosted models must receive the user's Off preference"
    );
    let prompt = body["messages"][0]["content"].as_str().unwrap();
    assert!(!prompt.contains("\"direction\""));
    assert!(prompt.contains("Do not format text to fit balloons"));
}
#[tokio::test]
async fn download_verifies_and_recovers_from_corrupt_target() {
    let server = MockServer::builder().start().await;
    let data = b"verified checkpoint";
    Mock::given(method("GET"))
        .and(path("/model"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(data.to_vec()))
        .expect(1)
        .mount(&server)
        .await;
    let root = tempfile::tempdir().unwrap();
    let pack = ModelPack {
        description: "Test pack".into(),
        distribution: "download".into(),
        id: "test".into(),
        name: "test".into(),
        kind: "test".into(),
        languages: vec![],
        license: "test".into(),
        revision: "pinned".into(),
        files: vec![ModelFile {
            name: "model.onnx".into(),
            url: format!("{}/model", server.uri()),
            sha256: store::digest(data),
            bytes: data.len() as u64,
        }],
    };
    std::fs::create_dir(root.path().join("test")).unwrap();
    std::fs::write(root.path().join("test/model.onnx"), b"corrupt").unwrap();
    assert!(!models::verify(root.path(), &pack).unwrap());
    models::download(root.path(), &pack, &CancellationToken::new(), |_, _| {})
        .await
        .unwrap();
    assert!(models::verify(root.path(), &pack).unwrap());
}
#[tokio::test]
async fn cancelled_download_never_installs_partial_checkpoint() {
    let root = tempfile::tempdir().unwrap();
    let token = CancellationToken::new();
    token.cancel();
    let pack = ModelPack {
        description: "Test pack".into(),
        distribution: "download".into(),
        id: "test".into(),
        name: "test".into(),
        kind: "test".into(),
        languages: vec![],
        license: "test".into(),
        revision: "pinned".into(),
        files: vec![ModelFile {
            name: "model.onnx".into(),
            url: "http://127.0.0.1:9/never".into(),
            sha256: "0".repeat(64),
            bytes: 8,
        }],
    };
    assert!(
        models::download(root.path(), &pack, &token, |_, _| {})
            .await
            .is_err()
    );
    assert!(!root.path().join("test/model.onnx").exists());
}
#[test]
fn appearance_masks_and_type_do_not_change_translation_key() {
    let mut page = Page {
        id: "p".into(),
        number: 0,
        name: "p".into(),
        source: Source {
            path: "p.png".into(),
            kind: "image".into(),
            entry: None,
            index: 0,
        },
        width: 100,
        height: 100,
        pdf_points: None,
        source_stamp: String::new(),
        fingerprint: "pixels".into(),
        revision: 0,
        regions: vec![Region::default()],
        rendered: None,
        background: None,
        cleanup: None,
        status: "new".into(),
        error: None,
    };
    let s = TranslationSettings::default();
    let p = ProviderProfile::default();
    let before = store::cache_key(&page, &s, &p, "").unwrap();
    page.regions[0].style.size = Some(30.);
    page.regions[0].style.fill = "#f00".into();
    page.regions[0].allow_fill = true;
    assert_eq!(before, store::cache_key(&page, &s, &p, "").unwrap());
    page.regions[0].source = "changed OCR".into();
    assert_ne!(before, store::cache_key(&page, &s, &p, "").unwrap());
}

#[tokio::test]
async fn partial_download_resumes_at_verified_offset() {
    let server = MockServer::builder().start().await;
    let data = b"a complete pinned checkpoint";
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("test");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("model.part"), &data[..9]).unwrap();
    Mock::given(method("GET"))
        .and(path("/model"))
        .and(header("range", "bytes=9-"))
        .respond_with(
            ResponseTemplate::new(206)
                .insert_header(
                    "content-range",
                    format!("bytes 9-{}/{}", data.len() - 1, data.len()),
                )
                .set_body_bytes(data[9..].to_vec()),
        )
        .expect(1)
        .mount(&server)
        .await;
    let pack = ModelPack {
        description: "Test pack".into(),
        distribution: "download".into(),
        id: "test".into(),
        name: "test".into(),
        kind: "test".into(),
        languages: vec![],
        license: "test".into(),
        revision: "pinned".into(),
        files: vec![ModelFile {
            name: "model.onnx".into(),
            url: format!("{}/model", server.uri()),
            sha256: store::digest(data),
            bytes: data.len() as u64,
        }],
    };
    models::download(root.path(), &pack, &CancellationToken::new(), |_, _| {})
        .await
        .unwrap();
    assert!(models::verify(root.path(), &pack).unwrap());
}

#[tokio::test]
async fn rate_limit_retries_once_and_partial_results_remain_partial() {
    let server = MockServer::builder().start().await;
    let profile = ProviderProfile {
        simple_translation: false,
        endpoint: format!("{}/v1", server.uri()),
        model: "test".into(),
        rate_limit: 0.5,
        ..Default::default()
    };
    let regions = vec![
        Region {
            id: "a".into(),
            source: "こんにちは".into(),
            ..Default::default()
        },
        Region {
            id: "b".into(),
            source: "読めない".into(),
            ..Default::default()
        },
    ];
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "1")
                .set_body_json(
                    serde_json::json!({"error":{"message":"rate limit","type":"rate_limit"}}),
                ),
        )
        .up_to_n_times(1)
        .expect(1)
        .with_priority(1)
        .mount(&server)
        .await;
    let content =
        serde_json::json!({"regions":[{"id":"a","target":"你好"},{"id":"b","target":""}]})
            .to_string();
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id":"mock","model":"test","choices":[{"index":0,"message":{"role":"assistant","content":content},"finish_reason":"stop"}]}))).expect(1).with_priority(2).mount(&server).await;
    let started = std::time::Instant::now();
    let results = providers::llm(
        &profile,
        "dummy",
        &TranslationSettings::default(),
        &regions,
        None,
        "",
    )
    .await
    .unwrap();
    assert!(
        started.elapsed() >= std::time::Duration::from_millis(1900),
        "Retry must obey the same HTTP-attempt limiter"
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "a");
}

#[test]
fn missing_ids_and_malformed_results_never_invent_a_translation() {
    let regions = vec![Region {
        id: "a".into(),
        ..Default::default()
    }];
    assert!(providers::validate("not JSON", &regions).is_err());
    assert!(providers::validate(r#"{"regions":[{"target":"fake"}]}"#, &regions).is_err());
    assert!(
        providers::validate(r#"{"regions":[]}"#, &regions)
            .unwrap()
            .is_empty()
    );
}
