use serde_json::json;
use std::time::Duration;
use umanga_core::{ollama, providers, setup, types::*};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, method, path},
};

async fn model(server: &MockServer, capabilities: &[&str], remote: bool) {
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"models":[{"name":"test:4b","size":100,"digest":"one"}]})),
        )
        .mount(server)
        .await;
    Mock::given(method("POST")).and(path("/api/show")).and(body_json(json!({"model":"test:4b","verbose":false}))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"capabilities":capabilities,"remote_host":if remote {"https://ollama.com"} else {""}}))).mount(server).await;
}
fn profile(server: &MockServer) -> ProviderProfile {
    ProviderProfile {
        simple_translation: false,
        protocol: "ollama".into(),
        endpoint: server.uri(),
        model: "test:4b".into(),
        vision: false,
        ..Default::default()
    }
}
#[test]
fn local_lan_and_ipv6_addresses_do_not_relax_hosted_credentials() {
    for host in [
        "http://localhost:11434/",
        "http://192.168.1.20:11434",
        "http://manga-pc:11434",
        "http://[::1]:11434",
        "https://ollama.lan/prefix/v1/",
    ] {
        let p = ProviderProfile {
            simple_translation: false,
            protocol: "ollama".into(),
            endpoint: host.into(),
            ..Default::default()
        };
        assert!(providers::credential_scope(&p).is_ok());
        assert!(!providers::credential_required(&p));
        assert!(providers::endpoint(&p).ends_with("/v1/"));
    }
    let mut p = ProviderProfile {
        simple_translation: false,
        protocol: "ollama".into(),
        endpoint: "http://192.168.1.20:11434".into(),
        ..Default::default()
    };
    p.protocol = "openai".into();
    assert!(providers::credential_scope(&p).is_err());
    for bad in [
        "ftp://localhost",
        "http://user:secret@localhost:11434",
        "http://localhost:11434?key=secret",
    ] {
        assert!(ollama::base_url(bad).is_err());
    }
    assert_eq!(ollama::base_url("").unwrap(), ollama::DEFAULT_SERVER);
    assert!(ollama::destination("http://[::1]:11434").starts_with("This computer"));
}
#[tokio::test]
async fn concurrent_metadata_is_deduplicated_and_never_generates() {
    let server = MockServer::builder().start().await;
    model(&server, &["completion", "vision"], false).await;
    let endpoint = server.uri();
    let (a, b) = tokio::join!(
        ollama::details(&endpoint, "test:4b", false),
        ollama::details(&endpoint, "test:4b", false)
    );
    assert!(a.connected && b.connected);
    assert!(a.model.unwrap().validate(true).is_ok());
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
    assert!(
        ollama::require_model(&server.uri(), "test:4b", true)
            .await
            .is_ok()
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
    let updated = ollama::details(&server.uri(), "test:4b", true).await;
    assert!(updated.connected);
    assert_eq!(server.received_requests().await.unwrap().len(), 4);
}
#[tokio::test]
async fn keyless_text_and_vision_use_existing_transport_with_strict_region_ids() {
    let server = MockServer::builder().start().await;
    model(&server, &["completion", "vision"], false).await;
    let p = profile(&server);
    assert!(setup::configuration(&TranslationSettings::default(), Some(&p), false).is_empty());
    let content =
        json!({"regions":[{"id":"setup-test","source":"123","target":"123"}]}).to_string();
    Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"mock","model":"test:4b","choices":[{"index":0,"message":{"role":"assistant","content":content},"finish_reason":"stop"}]}))).expect(2).mount(&server).await;
    let mut s = TranslationSettings {
        mode: "vision".into(),
        ..Default::default()
    };
    assert_eq!(setup::test_service(&p, "", &s).await.unwrap().target, "123");
    s.mode = "local".into();
    assert_eq!(setup::test_service(&p, "", &s).await.unwrap().target, "123");
    let requests = server.received_requests().await.unwrap();
    let chat: Vec<_> = requests
        .iter()
        .filter(|r| r.url.path().contains("chat/completions"))
        .collect();
    assert!(String::from_utf8_lossy(&chat[0].body).contains("data:image/"));
    assert!(!String::from_utf8_lossy(&chat[1].body).contains("data:image/"));
    let body: serde_json::Value = serde_json::from_slice(&chat[0].body).unwrap();
    assert_eq!(body["response_format"]["type"], "json_schema");
    assert_eq!(body["response_format"]["json_schema"]["strict"], true);
}
#[tokio::test]
async fn unsupported_cloud_missing_and_offline_models_never_reach_generation() {
    for (caps, remote, vision) in [
        (vec!["completion"], false, true),
        (vec!["embedding"], false, false),
        (vec!["completion", "vision"], true, false),
    ] {
        let server = MockServer::builder().start().await;
        model(&server, &caps, remote).await;
        ollama::details(&server.uri(), "test:4b", true).await;
        assert!(
            ollama::require_model(&server.uri(), "test:4b", vision)
                .await
                .is_err()
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
        assert!(
            ollama::require_model(&server.uri(), "not-installed", false)
                .await
                .is_err()
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }
    let offline = ollama::list("http://127.0.0.1:1", false).await;
    assert!(!offline.connected);
    assert!(offline.message.unwrap().contains("Cannot connect"));
    let server = MockServer::builder().start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"unexpected":true})))
        .mount(&server)
        .await;
    assert!(!ollama::list(&server.uri(), false).await.connected);
}
#[tokio::test]
async fn cancellation_drops_the_pending_ollama_request_without_retry() {
    let server = MockServer::builder().start().await;
    model(&server, &["completion"], false).await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(20)))
        .expect(1)
        .mount(&server)
        .await;
    let p = profile(&server);
    let settings = TranslationSettings {
        mode: "local".into(),
        ..Default::default()
    };
    let started = std::time::Instant::now();
    assert!(
        tokio::time::timeout(
            Duration::from_millis(400),
            setup::test_service(&p, "", &settings)
        )
        .await
        .is_err()
    );
    assert!(started.elapsed() < Duration::from_secs(2));
}
#[tokio::test]
async fn readiness_checks_installed_capabilities_and_invalidates_changed_inventory() {
    let server = MockServer::builder().start().await;
    model(&server, &["completion", "vision"], false).await;
    let p = profile(&server);
    let root = tempfile::tempdir().unwrap();
    let mut s = AppSettings::default();
    s.translation.mode = "vision".into();
    s.translation.cleanup.method = CleanupMethod::Solid;
    s.library_directory = root.path().to_string_lossy().into();
    s.translation.provider_id = p.id.clone();
    s.providers = vec![p];
    let locations = umanga_core::models::ModelLocations {
        bundled: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/models"),
        downloaded: std::path::PathBuf::new(),
    };
    let mut report =
        setup::readiness(&s, &locations, &mut Default::default(), false, false).unwrap();
    setup::check_service(&mut report, &s, false).await;
    assert!(report.ready && !report.credential_required && !report.credential_stored);
    assert!(setup::validate_step(&report, "review", true, "vision").is_ok());
    server.reset().await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models":[]})))
        .mount(&server)
        .await;
    setup::check_service(&mut report, &s, true).await;
    assert!(!report.ready);
    assert!(setup::validate_step(&report, "review", true, "vision").is_err());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_all_cancels_an_active_ollama_job_without_credentials_or_late_edits() {
    use std::sync::Arc;
    use umanga_core::{documents, pipeline::Engine, store};
    let server = MockServer::builder().start().await;
    model(&server, &["completion"], false).await;
    ollama::details(&server.uri(), "test:4b", true).await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(10)))
        .expect(1)
        .mount(&server)
        .await;
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = tempfile::tempdir_in(root.join("test-output")).unwrap();
    let image_path = dir.path().join("sample.png");
    image::RgbImage::from_pixel(40, 40, image::Rgb([255, 255, 255]))
        .save(&image_path)
        .unwrap();
    let mut page = documents::import(&[image_path.to_string_lossy().into()])
        .unwrap()
        .remove(0);
    page.regions = vec![Region {
        id: "kept".into(),
        source: "123".into(),
        bbox: [1., 1., 30., 30.],
        ..Default::default()
    }];
    let book = store::create(&dir.path().join("book.umanga"), "Test", &[page.clone()]).unwrap();
    store::settings(
        std::path::Path::new(&book.path),
        &TranslationSettings {
            mode: "local".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let e = Engine::new(
        root.join("assets/models"),
        &root.join("assets/fonts"),
        Arc::new(|_| {}),
        Arc::new(|_| anyhow::bail!("Credential lookup must never run for Ollama")),
    )
    .unwrap();
    e.enqueue(&book.path, &[page.id.clone()], &profile(&server))
        .unwrap();
    e.start();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .any(|r| r.url.path() == "/v1/chat/completions")
            {
                break;
            }
            if let Some(job) = e.list().first().filter(|j| j.status == "failed") {
                panic!("Job failed before Ollama: {:?}", job.error);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let start = std::time::Instant::now();
    e.control_all("stop").unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while e.snapshot(None).iter().any(|j| j.stopping) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(start.elapsed() < Duration::from_secs(2));
    assert_eq!(e.list()[0].status, "paused");
    let saved = store::page(std::path::Path::new(&book.path), &page.id).unwrap();
    assert!(saved.regions[0].target.is_empty());
    assert!(saved.rendered.is_none());
    e.control(&e.list()[0].id, "cancel").unwrap();
}
