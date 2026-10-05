use std::path::PathBuf;
#[path = "support/generation.rs"]
mod generation;
use umanga_core::{
    models::{self, ModelLocations, VerificationCache},
    setup,
    types::*,
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

fn configured() -> AppSettings {
    let mut settings = AppSettings::default();
    settings.translation.mode = "vision".into();
    settings.translation.cleanup.method = CleanupMethod::Solid;
    let profile = ProviderProfile {
        id: "chosen".into(),
        endpoint: "https://example.invalid/v1".into(),
        model: "explicit-image-model".into(),
        ..Default::default()
    };
    settings.translation.provider_id = profile.id.clone();
    settings.providers.push(profile);
    settings
}

#[test]
fn advancement_blocks_only_prerequisites_and_exposes_the_same_native_decision() {
    let root = tempfile::tempdir().unwrap();
    let mut s = AppSettings::default();
    let locations = ModelLocations {
        bundled: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/models"),
        downloaded: root.path().join("models"),
    };
    let mut cache = VerificationCache::default();
    let report = setup::readiness(&s, &locations, &mut cache, false, false).unwrap();
    assert!(!report.advancement["library"].ready);
    s.library_directory = root.path().to_string_lossy().into();
    let report = setup::readiness(&s, &locations, &mut cache, false, false).unwrap();
    for (step, destination) in [
        ("library", "pipeline"),
        ("pipeline", "services"),
        ("services", "models"),
        ("models", "review"),
    ] {
        assert_eq!(
            report.advancement[step].ready,
            setup::validate_step(&report, destination, false, "local").is_ok()
        );
    }
    assert!(report.advancement["library"].ready);
    assert!(report.advancement["pipeline"].ready);
    assert!(
        report.advancement["services"].ready,
        "Local manual translation requires no service"
    );
    assert!(!report.advancement["models"].ready);
    assert!(!report.advancement["review"].ready);
    let missing_detector = ModelLocations {
        bundled: root.path().join("missing"),
        downloaded: root.path().join("models"),
    };
    let report = setup::readiness(&s, &missing_detector, &mut cache, false, false).unwrap();
    assert!(report.advancement["library"].ready);
    assert!(!report.advancement["pipeline"].ready);
    s.translation.mode = "vision".into();
    s.translation.cleanup.method = CleanupMethod::Solid;
    let report = setup::readiness(&s, &locations, &mut cache, false, false).unwrap();
    assert!(!report.advancement.contains_key("models"));
    assert!(!report.advancement["services"].ready);
}
#[test]
fn gpu_defaults_preserve_independent_saved_device_choices() {
    let mut settings = AppSettings::default();
    assert_eq!(settings.translation.device, "directml");
    assert_eq!(settings.translation.cleanup.device, CleanupDevice::Directml);

    for ocr in ["cpu", "directml"] {
        for cleanup in [
            CleanupDevice::Cpu,
            CleanupDevice::Directml,
            CleanupDevice::Auto,
        ] {
            settings.translation.device = ocr.into();
            settings.translation.cleanup.device = cleanup;
            let stored = serde_json::to_vec(&settings).unwrap();
            let reopened: AppSettings = serde_json::from_slice(&stored).unwrap();
            assert_eq!(reopened.translation.device, ocr);
            assert_eq!(reopened.translation.cleanup.device, cleanup);
        }
    }
}
#[test]
fn vision_ready_offline_without_download_directory_or_service_test() {
    let root = tempfile::tempdir().unwrap();
    let mut settings = configured();
    settings.library_directory = root.path().to_string_lossy().into();
    let locations = ModelLocations {
        bundled: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/models"),
        downloaded: PathBuf::new(),
    };
    let mut cache = VerificationCache::default();
    let report = setup::readiness(&settings, &locations, &mut cache, true, false).unwrap();
    assert!(report.ready, "{:?}", report.issues);
    assert_eq!(
        report
            .packs
            .iter()
            .filter(|p| p.required)
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        vec!["rtdetr_int8"]
    );
    assert!(setup::validate_step(&report, "review", true, "vision").is_ok());
    let missing_key = setup::readiness(&settings, &locations, &mut cache, false, false).unwrap();
    assert!(!missing_key.ready);
    assert!(missing_key.issues.iter().any(|i| i.code == "credential"));
    // No network test result is required or implied by a saved credential.
    assert!(setup::validate_step(&missing_key, "review", true, "vision").is_err());
    assert!(setup::validate_step(&missing_key, "services", false, "vision").is_ok());
}
#[test]
fn language_and_mode_requirements_follow_effective_book_configuration() {
    let catalog = models::catalog().unwrap();
    let mut s = configured();
    s.translation.mode = "local".into();
    assert_eq!(
        setup::required_packs(&s.translation, &catalog),
        vec!["rtdetr_int8", "manga_ocr_onnx"]
    );
    s.translation.source_language = "ko".into();
    assert!(
        setup::configuration(&s.translation, s.providers.first(), true)
            .iter()
            .any(|i| i.code == "ocr_language")
    );
    s.translation.ocr = "pp".into();
    assert_eq!(
        setup::required_packs(&s.translation, &catalog),
        vec!["rtdetr_int8", "pp_det", "pp_korean"]
    );
    assert!(setup::configuration(&s.translation, s.providers.first(), true).is_empty());
    s.providers[0].service = "google".into();
    s.translation.mode = "vision".into();
    assert!(
        setup::configuration(&s.translation, s.providers.first(), true)
            .iter()
            .any(|i| i.code == "vision")
    );
    s.translation.mode = "local".into();
    assert!(setup::configuration(&s.translation, s.providers.first(), true).is_empty());
}
#[test]
fn missing_ocr_blocks_review_but_allows_earlier_steps_and_persists_step() {
    let root = tempfile::tempdir().unwrap();
    let mut s = configured();
    s.library_directory = root.path().to_string_lossy().into();
    s.translation.mode = "local".into();
    // The OCR engine is chosen on Read text; incompatibility blocks advancing to Services.
    s.translation.source_language = "ko".into();
    let locations = ModelLocations {
        bundled: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/models"),
        downloaded: PathBuf::new(),
    };
    let report = setup::readiness(
        &s,
        &locations,
        &mut VerificationCache::default(),
        true,
        false,
    )
    .unwrap();
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.code == "ocr_language")
    );
    assert!(setup::validate_step(&report, "services", false, "local").is_err());
    assert!(setup::validate_step(&report, "models", false, "local").is_err());
    assert!(setup::validate_step(&report, "review", false, "local").is_err());
    assert!(setup::validate_step(&report, "library", false, "local").is_ok());
    assert!(setup::validate_step(&report, "library", true, "local").is_err());
    s.setup.step = "models".into();
    let stored = serde_json::to_vec(&s).unwrap();
    let reopened: AppSettings = serde_json::from_slice(&stored).unwrap();
    assert_eq!(reopened.setup.step, "models");
    assert!(!reopened.setup.completed);
    let mut old = serde_json::to_value(&s).unwrap();
    old.as_object_mut().unwrap().remove("setup");
    assert!(serde_json::from_value::<AppSettings>(old).is_err());
}
#[test]
fn cached_verification_invalidates_missing_and_changed_files() {
    let root = tempfile::tempdir().unwrap();
    let mut pack = models::catalog().unwrap().remove(0);
    pack.files.truncate(1);
    pack.files[0].bytes = 2;
    pack.files[0].sha256 = umanga_core::store::digest(b"ok");
    let dir = root.path().join(&pack.id);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(&pack.files[0].name);
    let mut cache = VerificationCache::default();
    assert!(!cache.check(root.path(), &pack, false).unwrap());
    std::fs::write(&file, b"ok").unwrap();
    assert!(cache.check(root.path(), &pack, false).unwrap());
    assert_eq!(cache.cached(root.path(), &pack), Some(true));
    std::fs::write(&file, b"damaged").unwrap();
    assert!(!cache.check(root.path(), &pack, false).unwrap());
    std::fs::write(&file, b"ok").unwrap();
    assert!(cache.check(root.path(), &pack, true).unwrap());
    std::fs::remove_file(&file).unwrap();
    assert!(!cache.check(root.path(), &pack, false).unwrap());
}
#[tokio::test]
async fn explicit_service_tests_use_synthetic_sample_and_existing_transport_only() {
    let server = MockServer::builder().start().await;
    let p = ProviderProfile {
        endpoint: format!("{}/v1", server.uri()),
        simple_translation: false,
        model: "mock".into(),
        ..Default::default()
    };
    let payload =
        serde_json::json!({"regions":[{"id":"setup-test","source":"123","target":"123"}]});
    Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id":"mock","model":"mock","choices":[{"index":0,"message":{"role":"assistant","content":payload.to_string()},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}))).expect(2).mount(&server).await;
    let mut s = TranslationSettings {
        mode: "vision".into(),
        ..Default::default()
    };
    assert_eq!(
        setup::test_service(&p, "dummy-test-key", &s)
            .await
            .unwrap()
            .target,
        "123"
    );
    s.mode = "local".into();
    assert_eq!(
        setup::test_service(&p, "dummy-test-key", &s)
            .await
            .unwrap()
            .source,
        "123"
    );
    let requests = server.received_requests().await.unwrap();
    for request in &requests {
        generation::assert_service_defaults(
            &serde_json::from_slice(&request.body).unwrap(),
            "openai",
        );
    }
    let vision = String::from_utf8(requests[0].body.clone()).unwrap();
    let text = String::from_utf8(requests[1].body.clone()).unwrap();
    assert!(vision.contains("data:image/"));
    assert!(!text.contains("data:image/"));
    assert!(text.contains("setup-test"));
}
